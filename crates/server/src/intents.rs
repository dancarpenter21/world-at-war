use super::*;
use sim_core::{OrderKind, OrderResult, OrderStatus};

pub(super) type SubmissionResult = Result<SubmissionOutcome, (StatusCode, Json<ErrorResponse>)>;

pub(super) struct IntentSubmission {
    player_id: Uuid,
    role_id: Uuid,
    intent: PlayerIntent,
    submitted_tick: u64,
    result: SubmissionResult,
    execution: Option<(u64, OrderResult)>,
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
    if game.status != GameStatus::Running {
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
    let intent = request.intent;
    let submitted_tick = game.simulation.tick();
    let result = submit_authority_action(
        game,
        role_id,
        intent.kind.action_key().into(),
        intent.target,
        String::new(),
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
            execution: None,
        },
    );
    result
}

pub(super) fn record_execution_results(game: &mut Game) {
    let tick = game.simulation.tick();
    for result in game.simulation.drain_order_results() {
        if let Some(submission) = game.intent_submissions.get_mut(&result.intent_id) {
            submission.execution = Some((tick, result));
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
        record
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
    if let Some((tick, result)) = &submission.execution {
        receipt.executed_tick = Some(*tick);
        receipt.state = match &result.status {
            OrderStatus::Accepted => IntentState::Executed,
            OrderStatus::Rejected(reason) => {
                receipt.error = Some(reason.clone());
                IntentState::Rejected
            }
        };
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

pub(super) fn intent_fields(intent: &PlayerIntent) -> BTreeMap<String, serde_json::Value> {
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
        }
    }
    fields
}

#[cfg(test)]
mod tests;
