use super::*;
use sim_core::operations::*;
use sim_core::DeliveryState;

#[derive(Default)]
pub(super) struct PlanningState {
    template: Option<CampaignPlan>,
    pub(super) drafts: BTreeMap<Uuid, CampaignPlan>,
    published: Vec<CampaignPlan>,
    received: BTreeMap<Uuid, CampaignPlan>,
    reports: BTreeMap<Uuid, BTreeMap<Uuid, MissionReport>>,
    clearances: BTreeMap<Uuid, Vec<Clearance>>,
    handoffs: BTreeMap<Uuid, Vec<Handoff>>,
    proposals: BTreeMap<Uuid, Vec<Proposal>>,
}
#[derive(Clone, Serialize)]
struct Proposal {
    origin_role: Uuid,
    plan: CampaignPlan,
}
impl PlanningState {
    pub(super) fn new(template: Option<CampaignPlan>) -> Self {
        Self {
            template,
            ..Self::default()
        }
    }
}

#[derive(Serialize)]
pub(super) struct PlanningView {
    draft: Option<CampaignPlan>,
    received: Option<CampaignPlan>,
    comparisons: Vec<CourseComparison>,
    reports: Vec<MissionReport>,
    clearances: Vec<Clearance>,
    handoffs: Vec<Handoff>,
    tick: u64,
    proposals: Vec<Proposal>,
    assessments: Vec<ObjectiveAssessment>,
}
#[derive(Serialize)]
struct ObjectiveAssessment {
    objective_id: Uuid,
    description: String,
    observed_effect: bool,
    report_tick: Option<u64>,
}
#[derive(Deserialize)]
pub(super) struct PlanningRequest {
    player_id: Uuid,
    lease_generation: u64,
    #[serde(flatten)]
    action: PlanningAction,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PlanningAction {
    AdoptProposal {
        origin_role: Uuid,
        revision: u64,
    },
    Save {
        expected_revision: u64,
        plan: Box<CampaignPlan>,
    },
    Propose,
    Publish {
        expected_revision: u64,
        course_id: Uuid,
    },
    RequestClearance {
        clearance: Clearance,
    },
    GrantClearance {
        clearance: Clearance,
    },
    OfferHandoff {
        handoff: Handoff,
    },
    AcceptHandoff {
        handoff_id: Uuid,
    },
    Cancel {
        mission_ids: Vec<Uuid>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OperationMessage {
    Heartbeat {
        unit_id: Uuid,
        controller_role: Uuid,
    },
    Proposal {
        recipient_role: Uuid,
        plan: Box<CampaignPlan>,
    },
    Publication {
        recipient_role: Uuid,
        plan: Box<CampaignPlan>,
    },
    ClearanceRequest {
        recipient_role: Uuid,
        clearance: Clearance,
    },
    Clearance {
        recipient_role: Uuid,
        clearance: Clearance,
    },
    Handoff {
        recipient_role: Uuid,
        handoff: Handoff,
    },
    Cancel {
        unit_id: Uuid,
        mission_ids: Vec<Uuid>,
    },
    Report {
        recipient_role: Uuid,
        report: MissionReport,
    },
}

fn failure(message: impl Into<String>) -> (StatusCode, Json<ErrorResponse>) {
    api_error(
        StatusCode::UNPROCESSABLE_ENTITY,
        "planning_rejected",
        message,
    )
}

pub(super) fn view(game: &mut Game, role: &Role) -> PlanningView {
    let draft = game.planning.drafts.get(&role.id).cloned().or_else(|| {
        game.planning
            .template
            .as_ref()
            .filter(|p| p.commander_role_id == role.id)
            .cloned()
    });
    let received = game.planning.received.get(&role.id).cloned();
    let comparisons = draft
        .as_ref()
        .map(CampaignPlan::comparisons)
        .unwrap_or_default();
    let picture = game
        .simulation
        .projection_for(role.location_unit_id, role.side);
    let assessments = received
        .as_ref()
        .map(|plan| {
            plan.objectives
                .iter()
                .map(|objective| {
                    let evidence = picture
                        .tracks
                        .iter()
                        .filter(|track| {
                            track.assessed_destroyed == Some(true)
                                && (track.position.latitude_deg - objective.position.latitude_deg)
                                    .hypot(
                                        track.position.longitude_deg
                                            - objective.position.longitude_deg,
                                    )
                                    < 0.02
                        })
                        .max_by_key(|track| track.observed_tick);
                    ObjectiveAssessment {
                        objective_id: objective.id,
                        description: objective.description.clone(),
                        observed_effect: evidence.is_some(),
                        report_tick: evidence.map(|track| track.observed_tick),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let mut reports = game
        .planning
        .reports
        .get(&role.id)
        .map(|r| r.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for local in game.simulation.mission_reports(role.location_unit_id) {
        reports.retain(|r| r.mission_id != local.mission_id);
        reports.push(local);
    }
    PlanningView {
        assessments,
        draft,
        received,
        comparisons,
        reports,
        clearances: game
            .planning
            .clearances
            .get(&role.id)
            .cloned()
            .unwrap_or_default(),
        handoffs: game
            .planning
            .handoffs
            .get(&role.id)
            .cloned()
            .unwrap_or_default(),
        tick: game.simulation.tick(),
        proposals: game
            .planning
            .proposals
            .get(&role.id)
            .cloned()
            .unwrap_or_default(),
    }
}

pub(super) async fn get_planning(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<ProjectionQuery>,
) -> ApiResult<PlanningView> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| failure("game not found"))?;
    let role = authorized_role(game, &query)?.clone();
    Ok(Json(view(game, &role)))
}

pub(super) async fn update_planning(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    AuthJson(request): AuthJson<PlanningRequest>,
) -> ApiResult<PlanningView> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| failure("game not found"))?;
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    let role = game.roles[&role_id].clone();
    apply(game, &role, request.action).map_err(failure)?;
    Ok(Json(view(game, &role)))
}

fn role_at_unit(game: &Game, unit: Uuid) -> Option<Uuid> {
    game.roles
        .values()
        .find(|r| r.location_unit_id == unit && r.kind == AuthorityRoleKind::Pilot)
        .or_else(|| game.roles.values().find(|r| r.location_unit_id == unit))
        .map(|r| r.id)
}
fn send(
    game: &mut Game,
    origin: Uuid,
    destination: Uuid,
    operation: OperationMessage,
) -> Result<(), String> {
    let fields = BTreeMap::from([(
        "operation".into(),
        serde_json::to_value(&operation).map_err(|e| e.to_string())?,
    )]);
    transmit_c2_fields(
        game,
        origin,
        destination,
        "simulation.joint-operations.v1",
        "Joint operations message".into(),
        fields,
    )
    .ok_or_else(|| "no communication route to recipient".into())
    .map(|_| ())
}
pub(super) fn draft(game: &Game, role: Uuid) -> Result<CampaignPlan, String> {
    game.planning
        .drafts
        .get(&role)
        .or_else(|| game.planning.received.get(&role))
        .or_else(|| {
            game.planning
                .template
                .as_ref()
                .filter(|p| p.commander_role_id == role)
        })
        .cloned()
        .ok_or_else(|| "no planning product received".into())
}
fn known_plan(game: &Game, role: Uuid) -> Result<&CampaignPlan, String> {
    game.planning
        .received
        .get(&role)
        .ok_or_else(|| "no published plan received".into())
}
fn clearance_valid(game: &Game, role: Uuid, clearance: &Clearance) -> Result<(), String> {
    let plan = known_plan(game, role)?;
    let volume = plan
        .airspaces
        .iter()
        .find(|a| a.id == clearance.airspace_id)
        .ok_or("unknown airspace")?;
    if volume.controller_role_id != clearance.controller_role_id
        || !volume.covers(clearance.start_tick, clearance.end_tick)
    {
        return Err("clearance exceeds controller scope or time window".into());
    }
    let (_, ato) = plan.products()?;
    if !ato.missions.iter().any(|m| {
        m.unit_id == clearance.unit_id
            && m.start_tick < clearance.end_tick
            && clearance.start_tick < m.end_tick
    }) {
        return Err("clearance has no assigned mission".into());
    }
    Ok(())
}

fn apply(game: &mut Game, role: &Role, action: PlanningAction) -> Result<(), String> {
    match action {
        PlanningAction::AdoptProposal {
            origin_role,
            revision,
        } => {
            let current = draft(game, role.id)?;
            if current.commander_role_id != role.id {
                return Err("only the commander may adopt a proposal".into());
            }
            let mut proposed = game
                .planning
                .proposals
                .get(&role.id)
                .into_iter()
                .flatten()
                .find(|p| p.origin_role == origin_role && p.plan.revision == revision)
                .ok_or("proposal not received")?
                .plan
                .clone();
            proposed.revision = current.revision + 1;
            proposed.published_tick = None;
            proposed.validate(&game.authority, &game.unit_ids)?;
            game.planning.drafts.insert(role.id, proposed);
        }
        PlanningAction::Save {
            expected_revision,
            mut plan,
        } => {
            let existing = draft(game, role.id)?;
            if existing.revision != expected_revision {
                return Err("planning revision conflict; refresh before saving".into());
            }
            if plan.id != existing.id || plan.commander_role_id != existing.commander_role_id {
                return Err("plan identity and command appointment cannot be replaced".into());
            }
            if !matches!(
                role.kind,
                AuthorityRoleKind::JointForceCommander
                    | AuthorityRoleKind::CombatantCommander
                    | AuthorityRoleKind::ComponentCommander
            ) {
                return Err("role is not a planning participant".into());
            }
            for volume in &mut plan.airspaces {
                volume.normalize()?;
            }
            plan.revision = expected_revision + 1;
            plan.published_tick = None;
            plan.validate(&game.authority, &game.unit_ids)?;
            game.planning.drafts.insert(role.id, *plan);
        }
        PlanningAction::Propose => {
            let plan = draft(game, role.id)?;
            let recipient = plan.commander_role_id;
            let destination = game
                .roles
                .get(&recipient)
                .ok_or("commander missing")?
                .location_unit_id;
            send(
                game,
                role.id,
                destination,
                OperationMessage::Proposal {
                    recipient_role: recipient,
                    plan: Box::new(plan),
                },
            )?;
        }
        PlanningAction::Publish {
            expected_revision,
            course_id,
        } => {
            let mut plan = draft(game, role.id)?;
            if plan.commander_role_id != role.id {
                return Err("only the campaign commander may approve and publish".into());
            }
            if plan.revision != expected_revision {
                return Err("planning revision conflict".into());
            }
            if game
                .planning
                .published
                .iter()
                .any(|p| p.id == plan.id && p.revision >= plan.revision)
            {
                return Err("published revisions are immutable; save an amendment first".into());
            }
            plan.selected_course_id = Some(course_id);
            plan.validate(&game.authority, &game.unit_ids)?;
            let (_, ato) = plan.products()?;
            if ato
                .missions
                .iter()
                .any(|m| m.end_tick <= game.simulation.tick())
            {
                return Err("tasking window has already ended".into());
            }
            plan.published_tick = Some(game.simulation.tick());
            // All allied recipients receive the approved common coordination plan via their own links.
            let recipients: Vec<_> = game
                .roles
                .values()
                .filter(|r| r.side == role.side)
                .map(|r| (r.id, r.location_unit_id))
                .collect();
            if recipients.iter().any(|(_, destination)| {
                !game
                    .simulation
                    .has_message_route(role.location_unit_id, *destination)
            }) {
                return Err(
                    "publication has recipients without a configured communications route".into(),
                );
            }
            for (recipient_role, destination) in recipients {
                send(
                    game,
                    role.id,
                    destination,
                    OperationMessage::Publication {
                        recipient_role,
                        plan: Box::new(plan.clone()),
                    },
                )?;
            }
            game.planning.published.push(plan.clone());
            game.planning.drafts.insert(role.id, plan);
        }
        PlanningAction::RequestClearance { clearance } => {
            clearance_valid(game, role.id, &clearance)?;
            if !game
                .authority
                .role_is_in_unit_chain(role.id, clearance.unit_id)
                && role.location_unit_id != clearance.unit_id
            {
                return Err("cannot request clearance for this aircraft".into());
            }
            let recipient = clearance.controller_role_id;
            let destination = game
                .roles
                .get(&recipient)
                .ok_or("controller missing")?
                .location_unit_id;
            send(
                game,
                role.id,
                destination,
                OperationMessage::ClearanceRequest {
                    recipient_role: recipient,
                    clearance,
                },
            )?;
        }
        PlanningAction::GrantClearance { clearance } => {
            clearance_valid(game, role.id, &clearance)?;
            if clearance.controller_role_id != role.id {
                return Err("only the appointed controller may grant clearance".into());
            }
            let recipient =
                role_at_unit(game, clearance.unit_id).ok_or("aircraft has no operator role")?;
            send(
                game,
                role.id,
                clearance.unit_id,
                OperationMessage::Clearance {
                    recipient_role: recipient,
                    clearance,
                },
            )?;
        }
        PlanningAction::OfferHandoff { mut handoff } => {
            let plan = known_plan(game, role.id)?;
            let (_, ato) = plan.products()?;
            if !ato.missions.iter().any(|m| m.unit_id == handoff.unit_id) {
                return Err("handoff aircraft has no assigned mission".into());
            }
            let sector = plan
                .airspaces
                .iter()
                .find(|a| a.id == handoff.airspace_id)
                .ok_or("unknown destination sector")?;
            if role.id != handoff.from_role_id
                || sector.controller_role_id != handoff.to_role_id
                || !plan
                    .airspaces
                    .iter()
                    .any(|a| a.controller_role_id == role.id)
            {
                return Err("handoff exceeds controller appointment".into());
            }
            let destination = game
                .roles
                .get(&handoff.to_role_id)
                .ok_or("controller missing")?
                .location_unit_id;
            handoff.accepted = false;
            handoff.delivered = false;
            send(
                game,
                role.id,
                destination,
                OperationMessage::Handoff {
                    recipient_role: handoff.to_role_id,
                    handoff: handoff.clone(),
                },
            )?;
            let inbox = game.planning.handoffs.entry(role.id).or_default();
            inbox.retain(|h| h.id != handoff.id);
            inbox.push(handoff);
        }
        PlanningAction::AcceptHandoff { handoff_id } => {
            let mut handoff = game
                .planning
                .handoffs
                .get(&role.id)
                .into_iter()
                .flatten()
                .find(|h| h.id == handoff_id)
                .cloned()
                .ok_or("handoff not received")?;
            if handoff.to_role_id != role.id || handoff.accepted || handoff.delivered {
                return Err("handoff is not actionable".into());
            }
            handoff.accepted = true;
            let recipient_role =
                role_at_unit(game, handoff.unit_id).ok_or("aircraft operator missing")?;
            send(
                game,
                role.id,
                handoff.unit_id,
                OperationMessage::Handoff {
                    recipient_role,
                    handoff: handoff.clone(),
                },
            )?;
            let inbox = game.planning.handoffs.entry(role.id).or_default();
            inbox.retain(|h| h.id != handoff.id);
            inbox.push(handoff);
        }
        PlanningAction::Cancel { mission_ids } => {
            let plan = known_plan(game, role.id)?.clone();
            let (_, ato) = plan.products()?;
            let missions: Vec<_> = ato
                .missions
                .iter()
                .filter(|m| mission_ids.contains(&m.id))
                .collect();
            if missions.len() != mission_ids.len()
                || missions
                    .iter()
                    .any(|m| !game.authority.role_is_in_unit_chain(role.id, m.unit_id))
            {
                return Err("invalid cancellation scope".into());
            }
            for mission in missions {
                send(
                    game,
                    role.id,
                    mission.unit_id,
                    OperationMessage::Cancel {
                        unit_id: mission.unit_id,
                        mission_ids: vec![mission.id],
                    },
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn receive(game: &mut Game, payload: &[u8]) {
    let Ok(message) = serde_json::from_slice::<C2Message>(payload) else {
        return;
    };
    let Some(value) = message.fields.get("operation") else {
        return;
    };
    let Ok(operation) = serde_json::from_value::<OperationMessage>(value.clone()) else {
        return;
    };
    match operation {
        OperationMessage::Heartbeat {
            unit_id,
            controller_role,
        } => game
            .simulation
            .receive_controller_heartbeat(unit_id, controller_role),
        OperationMessage::Proposal {
            recipient_role,
            plan,
        } => {
            let proposals = game.planning.proposals.entry(recipient_role).or_default();
            proposals.retain(|p| p.origin_role != message.header.origin_role_id);
            proposals.push(Proposal {
                origin_role: message.header.origin_role_id,
                plan: *plan,
            });
        }
        OperationMessage::Publication {
            recipient_role,
            plan,
        } => {
            if plan.validate(&game.authority, &game.unit_ids).is_err() {
                return;
            }
            if game
                .planning
                .received
                .get(&recipient_role)
                .is_some_and(|old| old.revision >= plan.revision)
            {
                return;
            }
            if let (Some(role), Ok((aco, ato))) = (game.roles.get(&recipient_role), plan.products())
            {
                game.simulation
                    .receive_tasking(role.location_unit_id, &aco, &ato);
            }
            game.planning.received.insert(recipient_role, *plan);
        }
        OperationMessage::ClearanceRequest {
            recipient_role,
            clearance,
        } => {
            game.planning
                .clearances
                .entry(recipient_role)
                .or_default()
                .push(clearance);
        }
        OperationMessage::Clearance {
            recipient_role,
            clearance,
        } => {
            game.simulation.receive_clearance(clearance.clone());
            game.planning
                .clearances
                .entry(recipient_role)
                .or_default()
                .push(clearance);
        }
        OperationMessage::Handoff {
            recipient_role,
            mut handoff,
        } => {
            if handoff.accepted && message.header.recipient_entity_id == handoff.unit_id {
                let Some(plan) = game.planning.received.get(&recipient_role) else {
                    return;
                };
                let Some(volume) = plan.airspaces.iter().find(|a| a.id == handoff.airspace_id)
                else {
                    return;
                };
                game.simulation.receive_clearance(Clearance {
                    id: handoff.id,
                    airspace_id: volume.id,
                    unit_id: handoff.unit_id,
                    controller_role_id: handoff.to_role_id,
                    start_tick: volume.start_tick,
                    end_tick: volume.end_tick,
                });
                handoff.delivered = true;
                for controller in [handoff.from_role_id, handoff.to_role_id] {
                    if let Some(destination) =
                        game.roles.get(&controller).map(|r| r.location_unit_id)
                    {
                        let _ = send(
                            game,
                            recipient_role,
                            destination,
                            OperationMessage::Handoff {
                                recipient_role: controller,
                                handoff: handoff.clone(),
                            },
                        );
                    }
                }
            }
            let inbox = game.planning.handoffs.entry(recipient_role).or_default();
            inbox.retain(|h| h.id != handoff.id);
            inbox.push(handoff);
        }
        OperationMessage::Cancel {
            unit_id,
            mission_ids,
        } => game.simulation.cancel_missions(unit_id, &mission_ids),
        OperationMessage::Report {
            recipient_role,
            report,
        } => {
            if let Some(role) = game.roles.get(&recipient_role) {
                game.simulation
                    .receive_mission_report(role.location_unit_id, &report);
            }
            let reports = game.planning.reports.entry(recipient_role).or_default();
            if reports
                .get(&report.mission_id)
                .is_none_or(|old| old.observed_tick < report.observed_tick)
            {
                reports.insert(report.mission_id, report);
            }
        }
    }
}

pub(super) fn report_tick(game: &mut Game) {
    if !game.simulation.tick().is_multiple_of(5) {
        return;
    }
    let participants: Vec<_> = game
        .planning
        .received
        .iter()
        .filter_map(|(role_id, plan)| game.roles.get(role_id).map(|r| (r.clone(), plan.clone())))
        .collect();
    for (role, plan) in participants {
        if plan
            .airspaces
            .iter()
            .any(|a| a.controller_role_id == role.id)
        {
            if let Ok((_, ato)) = plan.products() {
                for unit_id in ato
                    .missions
                    .iter()
                    .map(|m| m.unit_id)
                    .collect::<BTreeSet<_>>()
                {
                    let _ = send(
                        game,
                        role.id,
                        unit_id,
                        OperationMessage::Heartbeat {
                            unit_id,
                            controller_role: role.id,
                        },
                    );
                }
            }
        }
        let reports = game.simulation.mission_reports(role.location_unit_id);
        for report in reports {
            let recipients: BTreeSet<_> = [plan.commander_role_id, plan.airspace_authority_role_id]
                .into_iter()
                .chain(plan.airspaces.iter().map(|a| a.controller_role_id))
                .chain(
                    plan.products()
                        .ok()
                        .into_iter()
                        .flat_map(|(_, ato)| ato.missions.into_iter())
                        .filter_map(|m| role_at_unit(game, m.unit_id)),
                )
                .collect();
            for recipient_role in recipients {
                if recipient_role == role.id {
                    continue;
                }
                if let Some(destination) =
                    game.roles.get(&recipient_role).map(|r| r.location_unit_id)
                {
                    let _ = send(
                        game,
                        role.id,
                        destination,
                        OperationMessage::Report {
                            recipient_role,
                            report: report.clone(),
                        },
                    );
                }
            }
        }
    }
}

fn transmit_c2_fields(
    game: &mut Game,
    origin_role_id: Uuid,
    recipient_entity_id: Uuid,
    profile_id: &str,
    rendered_text: String,
    fields: BTreeMap<String, serde_json::Value>,
) -> Option<Uuid> {
    let origin_entity_id = game
        .roles
        .get(&origin_role_id)
        .map(|role| role.location_unit_id)?;
    let message = C2Message {
        id: Uuid::from_u128(
            (u128::from(game.network_seed) << 64) | u128::from(game.network_event_sequence + 1),
        ),
        profile_id: profile_id.into(),
        header: MessageHeader {
            origin_role_id,
            origin_entity_id,
            recipient_entity_id,
            classification: "simulation-controlled".into(),
            priority: 230,
            created_tick: game.simulation.tick(),
            expires_tick: game.simulation.tick().saturating_add(300),
        },
        fields,
        rendered_text,
    };
    let encoded_bytes = message.encoded();
    let outcome = game.simulation.send_message(
        message.id,
        origin_entity_id,
        recipient_entity_id,
        encoded_bytes.clone(),
        message.header.expires_tick,
    );
    let (state, delivered_at_ns, drop_reason, mut delivered) = match outcome {
        Ok(()) => (MessageState::Queued, None, None, true),
        Err(error) => (MessageState::Dropped, None, Some(error.to_string()), false),
    };
    game.network_event_sequence = game.network_event_sequence.saturating_add(1);
    let mut record = NetworkMessageRecord {
        sequence: game.network_event_sequence,
        message,
        encoded_bytes,
        state,
        delivered_at_ns,
        packet_id: None,
        started_at_ns: None,
        terminal_at_ns: None,
        drop_reason,
    };
    if let Some(path) = &game.network_event_path {
        let persisted = serde_json::to_vec(&record).ok().and_then(|mut bytes| {
            bytes.push(b'\n');
            OpenOptions::new()
                .append(true)
                .open(path)
                .and_then(|mut file| file.write_all(&bytes))
                .ok()
        });
        if persisted.is_none() {
            delivered = false;
            record.state = MessageState::Dropped;
            record.drop_reason = Some("network event store write failed".into());
            game.status = GameStatus::Paused;
            game.operational_error = Some("network event store write failed; game paused".into());
        }
    }
    let message_id = record.message.id;
    game.network_message_events.push(record.clone());
    game.network_messages.push(record);
    delivered.then_some(message_id)
}

pub(super) fn process_deliveries(game: &mut Game) {
    for event in game.simulation.drain_deliveries() {
        let Some(mut record) = game
            .network_messages
            .iter()
            .find(|record| record.message.id == event.id)
            .cloned()
        else {
            continue;
        };
        record.state = match event.state {
            DeliveryState::Queued => MessageState::Queued,
            DeliveryState::Delivered => MessageState::Delivered,
            DeliveryState::Acknowledged => MessageState::Acknowledged,
            DeliveryState::Expired => MessageState::Expired,
            DeliveryState::Dropped => MessageState::Dropped,
            DeliveryState::Unacknowledged => MessageState::Unacknowledged,
        };
        if event.state == DeliveryState::Delivered {
            record.delivered_at_ns = Some(game.simulation.tick() * 1_000_000_000);
        }
        record.terminal_at_ns = Some(game.simulation.tick() * 1_000_000_000);
        if !transport::record_transition(game, record) {
            return;
        }
        if event.state == DeliveryState::Delivered {
            receive(game, &event.payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn game() -> Game {
        crate::authority_tests::game_for(regional_campaign_scenario())
    }
    fn advance(game: &mut Game, ticks: u64) {
        for _ in 0..ticks {
            game.simulation.step();
            process_deliveries(game);
            process_vacant_authority_requests(game);
            report_tick(game);
        }
    }

    #[test]
    fn publication_waits_for_each_recipient_and_rejects_non_commander() {
        let mut game = game();
        let commander = game.roles[&Uuid::from_u128(201)].clone();
        let pilot = game.roles[&Uuid::from_u128(211)].clone();
        let plan = draft(&game, commander.id).unwrap();
        assert!(apply(
            &mut game,
            &pilot,
            PlanningAction::Publish {
                expected_revision: 1,
                course_id: plan.courses[0].id
            }
        )
        .is_err());
        apply(
            &mut game,
            &commander,
            PlanningAction::Publish {
                expected_revision: 1,
                course_id: plan.courses[0].id,
            },
        )
        .unwrap();
        assert!(view(&mut game, &pilot).received.is_none());
        assert!(game
            .simulation
            .mission_reports(pilot.location_unit_id)
            .is_empty());
        advance(&mut game, 10);
        assert_eq!(view(&mut game, &pilot).received.unwrap().revision, 1);
        assert_eq!(
            game.simulation
                .mission_reports(pilot.location_unit_id)
                .len(),
            1
        );
        assert!(apply(
            &mut game,
            &commander,
            PlanningAction::Publish {
                expected_revision: 1,
                course_id: plan.courses[0].id
            }
        )
        .is_err());
    }

    #[test]
    fn stale_edits_and_wrong_sector_clearance_are_rejected() {
        let mut game = game();
        let commander = game.roles[&Uuid::from_u128(201)].clone();
        let west = game.roles[&Uuid::from_u128(205)].clone();
        let plan = draft(&game, commander.id).unwrap();
        assert!(apply(
            &mut game,
            &commander,
            PlanningAction::Save {
                expected_revision: 0,
                plan: Box::new(plan.clone())
            }
        )
        .is_err());
        apply(
            &mut game,
            &commander,
            PlanningAction::Publish {
                expected_revision: 1,
                course_id: plan.courses[0].id,
            },
        )
        .unwrap();
        advance(&mut game, 10);
        let east = &plan.airspaces[1];
        let clearance = Clearance {
            id: Uuid::from_u128(600),
            airspace_id: east.id,
            unit_id: Uuid::from_u128(11),
            controller_role_id: east.controller_role_id,
            start_tick: 10,
            end_tick: 900,
        };
        assert!(apply(
            &mut game,
            &west,
            PlanningAction::GrantClearance { clearance }
        )
        .is_err());
    }

    #[test]
    fn handoff_completes_only_after_acceptance_reaches_aircraft() {
        let mut game = game();
        let commander = game.roles[&Uuid::from_u128(201)].clone();
        let west = game.roles[&Uuid::from_u128(205)].clone();
        let east = game.roles[&Uuid::from_u128(206)].clone();
        let pilot = game.roles[&Uuid::from_u128(211)].clone();
        let plan = draft(&game, commander.id).unwrap();
        apply(
            &mut game,
            &commander,
            PlanningAction::Publish {
                expected_revision: 1,
                course_id: plan.courses[0].id,
            },
        )
        .unwrap();
        advance(&mut game, 10);
        assert!(apply(
            &mut game,
            &west,
            PlanningAction::OfferHandoff {
                handoff: Handoff {
                    id: Uuid::from_u128(602),
                    unit_id: Uuid::from_u128(61),
                    from_role_id: west.id,
                    to_role_id: east.id,
                    airspace_id: plan.airspaces[1].id,
                    accepted: false,
                    delivered: false,
                },
            },
        )
        .is_err());
        let handoff = Handoff {
            id: Uuid::from_u128(601),
            unit_id: pilot.location_unit_id,
            from_role_id: west.id,
            to_role_id: east.id,
            airspace_id: plan.airspaces[1].id,
            accepted: false,
            delivered: false,
        };
        apply(
            &mut game,
            &west,
            PlanningAction::OfferHandoff {
                handoff: handoff.clone(),
            },
        )
        .unwrap();
        assert!(view(&mut game, &east).handoffs.is_empty());
        advance(&mut game, 5);
        apply(
            &mut game,
            &east,
            PlanningAction::AcceptHandoff {
                handoff_id: handoff.id,
            },
        )
        .unwrap();
        assert!(view(&mut game, &east).handoffs[0].accepted);
        assert!(apply(
            &mut game,
            &east,
            PlanningAction::AcceptHandoff {
                handoff_id: handoff.id
            }
        )
        .is_err());
        assert!(view(&mut game, &pilot).handoffs.is_empty());
        advance(&mut game, 5);
        assert!(view(&mut game, &pilot).handoffs[0].delivered);
    }
}
