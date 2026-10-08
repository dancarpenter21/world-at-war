//! Atomic ACO-to-campaign draft integration. Preview and apply use identical parsing.
use super::*;
use sim_comms::aco::{self, Action, ImportOptions, Preview};
use sim_core::operations::*;
use sim_core::GeoPose;

#[derive(Deserialize)]
pub(super) struct ImportRequest {
    player_id: Uuid,
    lease_generation: u64,
    expected_revision: u64,
    source: String,
    options: ImportOptions,
}
#[derive(Serialize)]
pub(super) struct ImportPreview {
    #[serde(flatten)]
    parsed: Preview,
    changes: Vec<Change>,
    valid: bool,
    expected_revision: u64,
}
#[derive(Serialize)]
struct Change {
    external_id: String,
    action: String,
    airspace_id: Option<Uuid>,
}
fn rejected(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    api_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "airspace_import_rejected",
        message,
    )
}
fn prepare(
    game: &Game,
    role: &Role,
    request: &ImportRequest,
) -> Result<(ImportPreview, CampaignPlan), String> {
    if !matches!(
        role.kind,
        AuthorityRoleKind::JointForceCommander
            | AuthorityRoleKind::CombatantCommander
            | AuthorityRoleKind::ComponentCommander
    ) {
        return Err("role is not a planning participant".into());
    }
    let mut plan = planning::draft(game, role.id)?;
    if plan.revision != request.expected_revision {
        return Err("planning revision conflict; refresh before importing".into());
    }
    let mut parsed = aco::parse(&request.source, &request.options);
    let mut changes = vec![];
    for record in &mut parsed.records {
        if record.excluded {
            changes.push(Change {
                external_id: record.external_id.clone(),
                action: "excluded".into(),
                airspace_id: None,
            });
            continue;
        }
        let existing = plan.airspaces.iter().position(|a| {
            a.source.as_ref().is_some_and(|s| {
                s.order_id == parsed.order_id && s.external_id == record.external_id
            })
        });
        if existing.is_some_and(|i| {
            plan.airspaces[i]
                .source
                .as_ref()
                .is_some_and(|s| s.message_hash == parsed.source_hash)
        }) {
            changes.push(Change {
                external_id: record.external_id.clone(),
                action: "unchanged".into(),
                airspace_id: existing.map(|i| plan.airspaces[i].id),
            });
            continue;
        }
        let id = existing.map(|i| plan.airspaces[i].id);
        if matches!(record.action, Action::Change | Action::Cancel) && existing.is_none() {
            record
                .issues
                .push("amendment target does not exist in this campaign/order".into());
            continue;
        }
        if record.action == Action::Cancel {
            let id = id.unwrap();
            if plan
                .courses
                .iter()
                .flat_map(|c| &c.missions)
                .any(|m| m.clearance_airspace_ids.contains(&id))
            {
                record.issues.push(
                    "airspace is referenced by a mission; edit the mission before cancelling"
                        .into(),
                );
                continue;
            }
            plan.airspaces.remove(existing.unwrap());
            changes.push(Change {
                external_id: record.external_id.clone(),
                action: "cancel".into(),
                airspace_id: Some(id),
            });
            continue;
        }
        let controller = record
            .controller_role_id
            .as_ref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .filter(|id| game.roles.get(id).is_some_and(|r| r.side == role.side));
        if controller.is_none() {
            record
                .issues
                .push("controller must be an allied campaign role".into());
        }
        if !record.issues.is_empty() {
            continue;
        }
        let kind = match record.kind.as_str() {
            "sector" => AirspaceKind::Sector,
            "corridor" => AirspaceKind::Corridor,
            "restricted" => AirspaceKind::Restricted,
            "patrol" => AirspaceKind::Patrol,
            _ => continue,
        };
        let mut volume = AirspaceVolume {
            id: id.unwrap_or_else(Uuid::new_v4),
            name: record.name.clone(),
            kind,
            polygon: record
                .polygon
                .iter()
                .map(|p| GeoPose {
                    latitude_deg: p.latitude_deg,
                    longitude_deg: p.longitude_deg,
                    altitude_m: 0.0,
                })
                .collect(),
            floor_m: record.floor_m.unwrap(),
            ceiling_m: record.ceiling_m.unwrap(),
            source_geometry: record.geometry.clone(),
            active_periods: record
                .periods
                .iter()
                .map(|p| ActivationPeriod {
                    start_tick: p.start_tick,
                    end_tick: p.end_tick,
                })
                .collect(),
            start_tick: record.periods[0].start_tick,
            end_tick: record.periods.last().unwrap().end_tick,
            controller_role_id: controller.unwrap(),
            control_method: existing
                .map(|i| plan.airspaces[i].control_method)
                .unwrap_or(ControlMethod::Procedural),
            source: Some(AirspaceSource {
                order_id: parsed.order_id.clone(),
                external_id: record.external_id.clone(),
                message_hash: parsed.source_hash.clone(),
                raw: record.raw.clone(),
                resolutions: serde_json::to_value(&request.options).map_err(|e| e.to_string())?,
            }),
        };
        volume.normalize()?;
        changes.push(Change {
            external_id: record.external_id.clone(),
            action: if existing.is_some() {
                "change"
            } else {
                "create"
            }
            .into(),
            airspace_id: Some(volume.id),
        });
        if let Some(i) = existing {
            plan.airspaces[i] = volume
        } else {
            plan.airspaces.push(volume)
        }
    }
    if let Err(e) = plan.validate(&game.authority, &game.unit_ids) {
        parsed.issues.push(e);
    }
    let valid = parsed.issues.is_empty()
        && parsed
            .records
            .iter()
            .all(|r| r.excluded || r.issues.is_empty());
    Ok((
        ImportPreview {
            parsed,
            changes,
            valid,
            expected_revision: request.expected_revision,
        },
        plan,
    ))
}
pub(super) async fn preview(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    AuthJson(request): AuthJson<ImportRequest>,
) -> ApiResult<ImportPreview> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| rejected("game not found"))?;
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    let (preview, _) = prepare(game, &game.roles[&role_id], &request).map_err(rejected)?;
    Ok(Json(preview))
}
pub(super) async fn apply(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    AuthJson(request): AuthJson<ImportRequest>,
) -> ApiResult<planning::PlanningView> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| rejected("game not found"))?;
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    let role = game.roles[&role_id].clone();
    let (preview, mut plan) = prepare(game, &role, &request).map_err(rejected)?;
    if !preview.valid {
        return Err(rejected(
            "resolve or explicitly exclude every invalid record before applying",
        ));
    }
    if preview
        .changes
        .iter()
        .any(|c| c.action != "unchanged" && c.action != "excluded")
    {
        plan.revision += 1;
        plan.published_tick = None;
        game.planning.drafts.insert(role.id, plan);
    }
    Ok(Json(planning::view(game, &role)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> ImportRequest {
        ImportRequest {
            player_id: Uuid::nil(),
            lease_generation: 0,
            expected_revision: 1,
            source: include_str!("../../../data/aco/labelled.aco").into(),
            options: ImportOptions {
                anchor_utc: "2026-09-01T00:00:00Z".into(),
                anchor_tick: 0,
                year: 2026,
                horizon_end_utc: "2026-09-03T00:00:00Z".into(),
                resolutions: BTreeMap::from([(
                    "TRAINING".into(),
                    aco::Resolution {
                        controller_role_id: Some(Uuid::from_u128(205).to_string()),
                        kind: Some("patrol".into()),
                        ..Default::default()
                    },
                )]),
            },
        }
    }
    #[test]
    fn import_preserves_identity_and_draft_is_not_published() {
        let mut game = authority_tests::game_for(regional_campaign_scenario());
        let role = game.roles[&Uuid::from_u128(201)].clone();
        let req = request();
        let (preview, plan) = prepare(&game, &role, &req).unwrap();
        assert!(preview.valid, "{:?}", preview.parsed.issues);
        assert_eq!(plan.airspaces.len(), 3);
        let id = plan.airspaces[2].id;
        game.planning.drafts.insert(role.id, plan);
        let (preview, plan) = prepare(&game, &role, &req).unwrap();
        assert_eq!(preview.changes[0].action, "unchanged");
        assert_eq!(plan.airspaces[2].id, id);
        let pilot = game.roles[&Uuid::from_u128(211)].clone();
        assert!(planning::draft(&game, pilot.id).is_err());
    }
    #[test]
    fn invalid_import_and_stale_revision_do_not_mutate() {
        let game = authority_tests::game_for(regional_campaign_scenario());
        let role = game.roles[&Uuid::from_u128(201)].clone();
        let mut req = request();
        req.options.resolutions.clear();
        assert!(!prepare(&game, &role, &req).unwrap().0.valid);
        req.expected_revision = 99;
        assert!(prepare(&game, &role, &req).is_err());
        assert!(game.planning.drafts.is_empty());
    }
}
