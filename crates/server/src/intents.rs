use super::*;
use sim_core::{OrderKind, OrderStatus};

pub(super) type SubmissionResult = Result<SubmissionOutcome, (StatusCode, Json<ErrorResponse>)>;

pub(super) struct IntentSubmission {
    player_id: Uuid,
    role_id: Uuid,
    intent: PlayerIntent,
    submitted_tick: u64,
    result: SubmissionResult,
}

#[derive(Deserialize)]
pub(super) struct ReceiptQuery {
    player_id: Uuid,
    lease_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum IntentState {
    Queued,
    InTransit,
    AwaitingAuthority,
    AwaitingExecution,
    AwaitingAcknowledgement,
    Unconfirmed,
    Executed,
    Rejected,
    Dropped,
    Expired,
    Denied,
    ApprovedNoExecutor,
}

#[derive(Serialize)]
pub(super) struct IntentReceipt {
    intent: PlayerIntent,
    state: IntentState,
    submitted_tick: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    executed_tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    acknowledged_tick: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(super) fn submit_player_intent(
    game: &mut Game,
    role_id: Uuid,
    request: SubmitIntentRequest,
) -> SubmissionResult {
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    if request.intent.issuer_role != role_id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "issuer_role",
            "intent issuer does not match the held role",
        ));
    }
    if let Some(existing) = game.intent_submissions.get(&request.intent.intent_id) {
        if existing.player_id != request.player_id
            || existing.role_id != role_id
            || existing.intent != request.intent
        {
            return Err(api_error(
                StatusCode::CONFLICT,
                "intent_conflict",
                "this order ID has already been used for a different command",
            ));
        }
        return existing.result.clone();
    }
    if game.status != GameStatus::Running || game.simulation.mission_complete() {
        return Err(api_error(
            StatusCode::CONFLICT,
            "game_not_running",
            "game is not running",
        ));
    }
    if let OrderKind::Move {
        north_mps,
        east_mps,
    } = request.intent.kind
    {
        if !north_mps.is_finite() || !east_mps.is_finite() || north_mps.hypot(east_mps) > 1_000.0 {
            return Err(api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_movement",
                "movement speed must be finite and no greater than 1,000 m/s",
            ));
        }
    }
    if let OrderKind::Engage { track_id } = request.intent.kind {
        let terminal = game.roles[&role_id].location_unit_id;
        game.simulation
            .designate_engagement(
                request.intent.intent_id,
                request.intent.target,
                terminal,
                track_id,
            )
            .map_err(|error| {
                api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_engagement",
                    error,
                )
            })?;
    }
    let intent = request.intent;
    let submitted_tick = game.simulation.tick();
    let summary = game.simulation.engagement_designation(intent.intent_id).map(|report| format!(
        "Engage reported {:?} contact: {:.0}% identity, {:.5}°, {:.5}°, {:.0} m altitude; observed at tick {}.",
        report.target_side, report.identity_confidence * 100.0, report.position.latitude_deg,
        report.position.longitude_deg, report.position.altitude_m, report.observed_tick
    )).unwrap_or_default();
    let result = submit_authority_action(
        game,
        role_id,
        intent.kind.action_key().into(),
        intent.target,
        summary,
        Some(intent.clone()),
    );
    game.intent_submissions.insert(
        intent.intent_id,
        IntentSubmission {
            player_id: request.player_id,
            role_id,
            intent,
            submitted_tick,
            result: result.clone(),
        },
    );
    result
}

pub(super) fn record_execution_results(game: &mut Game) {
    let tick = game.simulation.tick();
    for result in game.simulation.drain_order_results() {
        if let Some(submission) = game.intent_submissions.get(&result.intent_id) {
            execution_acks::record_result(
                game,
                submission.role_id,
                submission.intent.target,
                tick,
                result,
            );
        }
    }
}

fn message_state(record: Option<&NetworkMessageRecord>) -> IntentState {
    match record.map(|record| record.state) {
        Some(MessageState::InTransit) => IntentState::InTransit,
        Some(MessageState::Delivered) => IntentState::AwaitingExecution,
        Some(MessageState::Dropped) => IntentState::Dropped,
        Some(MessageState::Expired) => IntentState::Expired,
        _ => IntentState::Queued,
    }
}

pub(super) fn receipt_for(
    game: &Game,
    role_id: Uuid,
    intent_id: Uuid,
    query: &ReceiptQuery,
) -> Result<IntentReceipt, (StatusCode, Json<ErrorResponse>)> {
    validate_role_lease(game, role_id, query.player_id, query.lease_generation)?;
    let submission = game
        .intent_submissions
        .get(&intent_id)
        .filter(|submission| {
            submission.player_id == query.player_id && submission.role_id == role_id
        })
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "intent_not_found", "order not found"))?;
    let intent_key = intent_id.to_string();
    let latest_message = game.network_messages.iter().rev().find(|record| {
        record.message.profile_id != execution_acks::ACK_PROFILE
            && record.message.profile_id != impact_reports::IMPACT_REPORT_PROFILE
            && record
                .message
                .fields
                .get("intent_id")
                .is_some_and(|id| id.as_str() == Some(intent_key.as_str()))
    });
    let mut receipt = IntentReceipt {
        intent: submission.intent.clone(),
        state: IntentState::Queued,
        submitted_tick: submission.submitted_tick,
        message_id: latest_message.map(|record| record.message.id),
        request_id: None,
        executed_tick: None,
        acknowledged_tick: None,
        error: None,
    };
    match &submission.result {
        Err((_, Json(error))) => {
            receipt.state = if error.code == "blocked_comms" {
                IntentState::Dropped
            } else {
                IntentState::Rejected
            };
            receipt.error = Some(error.error.clone());
        }
        Ok(SubmissionOutcome::Queued { message_id, .. }) => {
            receipt.message_id = Some(*message_id);
            receipt.state = message_state(
                game.network_messages
                    .iter()
                    .find(|record| record.message.id == *message_id),
            );
        }
        Ok(SubmissionOutcome::PendingAuthority { request_id, .. }) => {
            receipt.request_id = Some(*request_id);
            receipt.state = match game
                .authority_requests
                .get(request_id)
                .map(|request| &request.status)
            {
                Some(AuthorityRequestStatus::InTransit { message_id }) => {
                    receipt.message_id = Some(*message_id);
                    message_state(
                        game.network_messages
                            .iter()
                            .find(|record| record.message.id == *message_id),
                    )
                }
                Some(
                    AuthorityRequestStatus::PendingHuman { .. }
                    | AuthorityRequestStatus::WaitingVacant { .. }
                    | AuthorityRequestStatus::PendingExternal { .. },
                ) => IntentState::AwaitingAuthority,
                Some(AuthorityRequestStatus::Approved) => IntentState::AwaitingExecution,
                Some(AuthorityRequestStatus::ApprovedNoExecutor) => IntentState::ApprovedNoExecutor,
                Some(
                    AuthorityRequestStatus::Denied { .. }
                    | AuthorityRequestStatus::DeniedExternal { .. },
                ) => IntentState::Denied,
                Some(AuthorityRequestStatus::BlockedComms) => {
                    if message_state(latest_message) == IntentState::Expired {
                        IntentState::Expired
                    } else {
                        IntentState::Dropped
                    }
                }
                None => IntentState::Rejected,
            };
        }
    }
    if matches!(receipt.state, IntentState::Dropped | IntentState::Expired) {
        receipt.error = receipt
            .error
            .or_else(|| latest_message.and_then(|record| record.drop_reason.clone()));
    }
    if receipt.state == IntentState::AwaitingExecution
        && game.roles[&role_id].location_unit_id != submission.intent.target
    {
        receipt.state = IntentState::AwaitingAcknowledgement;
    }
    if let Some((tick, received_tick, result)) = execution_acks::confirmation(game, intent_id) {
        receipt.executed_tick = Some(tick);
        receipt.acknowledged_tick = Some(received_tick);
        receipt.state = match &result.status {
            OrderStatus::Accepted => IntentState::Executed,
            OrderStatus::Rejected(reason) => {
                receipt.error = Some(reason.clone());
                IntentState::Rejected
            }
        };
    }
    if receipt.state == IntentState::AwaitingAcknowledgement {
        let delivered_tick = latest_message
            .and_then(|record| record.delivered_at_ns)
            .map(|at| at.div_ceil(1_000_000_000));
        if (game.simulation.mission_complete() && game.status == GameStatus::Paused)
            || delivered_tick.is_some_and(|delivered| {
                game.simulation.radio_tick().saturating_sub(delivered)
                    >= execution_acks::confirmation_timeout(game)
            })
        {
            receipt.state = IntentState::Unconfirmed;
            receipt.error = Some("execution has not been confirmed over the radio".into());
        }
    }
    if game.simulation.mission_complete()
        && matches!(
            receipt.state,
            IntentState::Queued
                | IntentState::InTransit
                | IntentState::AwaitingAuthority
                | IntentState::AwaitingExecution
        )
    {
        receipt.state = IntentState::Rejected;
        receipt.error = Some("the training mission ended before this order executed".into());
    }
    Ok(receipt)
}

pub(super) async fn get_intent_receipt(
    Path((game_id, role_id, intent_id)): Path<(Uuid, Uuid, Uuid)>,
    State(state): State<AppState>,
    Query(query): Query<ReceiptQuery>,
) -> ApiResult<IntentReceipt> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    Ok(Json(receipt_for(game, role_id, intent_id, &query)?))
}

pub(super) fn intent_fields(
    intent: &PlayerIntent,
    simulation: &Simulation,
) -> BTreeMap<String, serde_json::Value> {
    let mut fields = BTreeMap::from([
        ("intent_id".into(), serde_json::json!(intent.intent_id)),
        (
            "requested_tick".into(),
            serde_json::json!(intent.requested_tick),
        ),
    ]);
    match &intent.kind {
        OrderKind::Move {
            north_mps,
            east_mps,
        } => {
            fields.insert("north_mps".into(), serde_json::json!(north_mps));
            fields.insert("east_mps".into(), serde_json::json!(east_mps));
        }
        OrderKind::Engage { track_id } => {
            fields.insert("track_id".into(), serde_json::json!(track_id));
            if let Some(report) = simulation.engagement_designation(intent.intent_id) {
                fields.insert("aim_point".into(), serde_json::json!(report.position));
                fields.insert(
                    "observed_tick".into(),
                    serde_json::json!(report.observed_tick),
                );
            }
        }
    }
    fields
}

#[cfg(test)]
mod tests;

#[derive(Serialize)]
pub(super) struct DebriefRadioLeg {
    profile_id: String,
    state: MessageState,
    sent_tick: u64,
    delivered_tick: Option<u64>,
}
#[derive(Serialize)]
pub(super) struct DebriefEntry {
    intent_id: Uuid,
    platform_id: Uuid,
    submitted_tick: Option<u64>,
    observed_tick: Option<u64>,
    approval_ticks: Vec<u64>,
    launch_tick: Option<u64>,
    acknowledged_tick: Option<u64>,
    impact_tick: Option<u64>,
    report_received_tick: Option<u64>,
    hit: Option<bool>,
    state: String,
    error: Option<String>,
    radio_legs: Vec<DebriefRadioLeg>,
}
#[derive(Serialize)]
pub(super) struct MissionDebrief {
    tick: u64,
    radio_tick: u64,
    settling_reports: bool,
    mission: Option<sim_core::combat::MissionOutcome>,
    entries: Vec<DebriefEntry>,
}

pub(super) fn mission_debrief(game: &mut Game, role: &Role) -> MissionDebrief {
    let combat = game
        .simulation
        .projection_for(role.location_unit_id, role.side)
        .combat;
    let local = game.simulation.local_impact_reports(role.location_unit_id);
    let mut entries = Vec::new();
    for submission in game
        .intent_submissions
        .values()
        .filter(|submission| matches!(submission.intent.kind, OrderKind::Engage { .. }))
    {
        let intent_id = submission.intent.intent_id;
        let request = game.authority_requests.values().find(|request| {
            request
                .intent
                .as_ref()
                .is_some_and(|intent| intent.intent_id == intent_id)
        });
        let own = submission.role_id == role.id;
        if !own && !request.is_some_and(|request| authority_request_known(request, role.id)) {
            continue;
        }
        let received = combat.as_ref().and_then(|combat| {
            combat
                .received_impacts
                .iter()
                .find(|impact| impact.report.intent_id == intent_id)
        });
        let report = local
            .iter()
            .find(|report| report.intent_id == intent_id)
            .or_else(|| received.map(|received| &received.report));
        let key = intent_id.to_string();
        let records: Vec<_> = game
            .network_messages
            .iter()
            .filter(|record| {
                network_message_visible(record, role)
                    && record
                        .message
                        .fields
                        .get("intent_id")
                        .is_some_and(|id| id.as_str() == Some(key.as_str()))
            })
            .collect();
        let receipt = own
            .then(|| {
                receipt_for(
                    game,
                    role.id,
                    intent_id,
                    &ReceiptQuery {
                        player_id: role.owner.expect("authorized held role"),
                        lease_generation: role.lease_generation,
                    },
                )
                .ok()
            })
            .flatten();
        let state = receipt
            .as_ref()
            .map(|receipt| {
                serde_json::to_value(receipt.state)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .unwrap_or_else(|| {
                if report.is_some() {
                    "reported".into()
                } else {
                    request
                        .map(|request| {
                            serde_json::to_value(&request.status).unwrap()["state"]
                                .as_str()
                                .unwrap()
                                .to_owned()
                        })
                        .unwrap_or_else(|| "unknown".into())
                }
            });
        entries.push(DebriefEntry {
            intent_id,
            platform_id: submission.intent.target,
            submitted_tick: Some(submission.submitted_tick),
            observed_tick: report.map(|report| report.observed_tick).or_else(|| {
                records.iter().find_map(|record| {
                    record
                        .message
                        .fields
                        .get("observed_tick")
                        .and_then(|tick| tick.as_u64())
                })
            }),
            approval_ticks: request
                .map(|request| {
                    request
                        .decisions
                        .iter()
                        .filter(|decision| decision.approved)
                        .map(|decision| decision.tick)
                        .collect()
                })
                .unwrap_or_default(),
            launch_tick: report.map(|report| report.launched_tick).or_else(|| {
                receipt
                    .as_ref()
                    .filter(|receipt| receipt.state == IntentState::Executed)
                    .and_then(|receipt| receipt.executed_tick)
            }),
            impact_tick: report.map(|report| report.resolved_tick),
            acknowledged_tick: receipt
                .as_ref()
                .and_then(|receipt| receipt.acknowledged_tick),
            report_received_tick: received.map(|received| received.received_tick),
            hit: report.map(|report| report.hit),
            error: receipt.and_then(|receipt| receipt.error),
            state,
            radio_legs: records
                .into_iter()
                .map(|record| DebriefRadioLeg {
                    profile_id: record.message.profile_id.clone(),
                    state: record.state,
                    sent_tick: record.message.header.created_tick,
                    delivered_tick: record
                        .delivered_at_ns
                        .map(|time| time.div_ceil(1_000_000_000)),
                })
                .collect(),
        });
    }
    // A received report can teach this terminal about a shot it did not issue or approve.
    // Build those rows solely from the report and visible packets, not the submission ledger.
    let mut known_impacts: Vec<_> = local
        .iter()
        .map(|report| (report.clone(), None, role.location_unit_id))
        .collect();
    if let Some(combat) = &combat {
        for received in &combat.received_impacts {
            let source = game
                .network_messages
                .iter()
                .find(|record| {
                    record.state == MessageState::Delivered
                        && record.message.profile_id == impact_reports::IMPACT_REPORT_PROFILE
                        && record.message.header.recipient_entity_id == role.location_unit_id
                        && record.message.fields.get("intent_id").is_some_and(|id| {
                            id.as_str() == Some(received.report.intent_id.to_string().as_str())
                        })
                })
                .map(|record| record.message.header.origin_entity_id);
            if let Some(source) = source {
                known_impacts.push((
                    received.report.clone(),
                    Some(received.received_tick),
                    source,
                ));
            }
        }
    }
    for (report, received_tick, platform_id) in known_impacts {
        if entries
            .iter()
            .any(|entry| entry.intent_id == report.intent_id)
        {
            continue;
        }
        let key = report.intent_id.to_string();
        let radio_legs = game
            .network_messages
            .iter()
            .filter(|record| {
                network_message_visible(record, role)
                    && record
                        .message
                        .fields
                        .get("intent_id")
                        .is_some_and(|id| id.as_str() == Some(key.as_str()))
            })
            .map(|record| DebriefRadioLeg {
                profile_id: record.message.profile_id.clone(),
                state: record.state,
                sent_tick: record.message.header.created_tick,
                delivered_tick: record
                    .delivered_at_ns
                    .map(|time| time.div_ceil(1_000_000_000)),
            })
            .collect();
        entries.push(DebriefEntry {
            intent_id: report.intent_id,
            platform_id,
            submitted_tick: None,
            observed_tick: Some(report.observed_tick),
            approval_ticks: vec![],
            launch_tick: Some(report.launched_tick),
            acknowledged_tick: None,
            impact_tick: Some(report.resolved_tick),
            report_received_tick: received_tick,
            hit: Some(report.hit),
            state: "reported".into(),
            error: None,
            radio_legs,
        });
    }
    entries.sort_by_key(|entry| (entry.submitted_tick.or(entry.launch_tick), entry.intent_id));
    MissionDebrief {
        tick: game.simulation.tick(),
        radio_tick: game.simulation.radio_tick(),
        settling_reports: game.simulation.mission_complete() && game.status == GameStatus::Running,
        mission: combat.and_then(|combat| combat.mission),
        entries,
    }
}

pub(super) async fn get_mission_debrief(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    Query(query): Query<ReceiptQuery>,
) -> ApiResult<MissionDebrief> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    validate_role_lease(game, role_id, query.player_id, query.lease_generation)?;
    let role = game.roles[&role_id].clone();
    Ok(Json(mission_debrief(game, &role)))
}
