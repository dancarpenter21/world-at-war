//! Execution knowledge returns from the executor only through a recorded acknowledgement.
use super::*;
use sim_core::OrderResult;

pub(super) const ACK_PROFILE: &str = "public-safe.network.ack.v1";
pub(super) const RETRY_TICKS: u64 = 10;
const MAX_ATTEMPTS: u8 = 4;

#[derive(Default)]
pub(super) struct ExecutionAckState {
    results: BTreeMap<Uuid, ExecutionAcknowledgement>,
}

struct ExecutionAcknowledgement {
    issuer_role_id: Uuid,
    executor_entity_id: Uuid,
    recipient_entity_id: Uuid,
    executed_tick: u64,
    result: OrderResult,
    attempts: u8,
    last_attempt_tick: Option<u64>,
    received_tick: Option<u64>,
}

pub(super) fn record_result(
    game: &mut Game,
    issuer_role_id: Uuid,
    executor_entity_id: Uuid,
    executed_tick: u64,
    result: OrderResult,
) {
    let recipient_entity_id = game.roles[&issuer_role_id].location_unit_id;
    game.execution_acks
        .results
        .entry(result.intent_id)
        .or_insert(ExecutionAcknowledgement {
            issuer_role_id,
            executor_entity_id,
            recipient_entity_id,
            executed_tick,
            result,
            attempts: 0,
            last_attempt_tick: None,
            received_tick: None,
        });
}

fn pending(game: &Game, intent_id: Uuid) -> bool {
    game.pending_deliveries.values().any(|action| {
        matches!(action, DeliveryAction::ReceiveExecutionAck { intent_id: pending } if *pending == intent_id)
    })
}

pub(super) fn settled(game: &Game) -> bool {
    game.execution_acks.results.iter().all(|(id, ack)| {
        ack.received_tick.is_some() || (ack.attempts >= MAX_ATTEMPTS && !pending(game, *id))
    })
}

pub(super) fn send_pending(game: &mut Game) {
    if game.status != GameStatus::Running || game.execution_acks.results.is_empty() {
        return;
    }
    if settled(game) {
        return;
    }
    if !game.message_profiles.contains_key(ACK_PROFILE) {
        game.status = GameStatus::Paused;
        game.operational_error =
            Some("acknowledgement message profile is missing; game paused".into());
        return;
    }
    let tick = game.simulation.radio_tick();
    let due: Vec<_> = game
        .execution_acks
        .results
        .iter()
        .filter_map(|(id, ack)| {
            (ack.received_tick.is_none()
                && ack.attempts < MAX_ATTEMPTS
                && !pending(game, *id)
                && ack
                    .last_attempt_tick
                    .is_none_or(|last| tick.saturating_sub(last) >= RETRY_TICKS))
            .then_some(*id)
        })
        .collect();
    for intent_id in due {
        let ack = game.execution_acks.results.get_mut(&intent_id).unwrap();
        ack.attempts += 1;
        ack.last_attempt_tick = Some(tick);
        let fields = BTreeMap::from([
            ("intent_id".into(), serde_json::json!(intent_id)),
            ("executed_tick".into(), serde_json::json!(ack.executed_tick)),
            ("result".into(), serde_json::json!(ack.result)),
        ]);
        // An unmanned executor can reply on behalf of the issuing authority;
        // the physical origin is always the executing unit, never the issuer's terminal.
        let origin_role_id = game
            .roles
            .values()
            .find(|role| role.location_unit_id == ack.executor_entity_id)
            .map_or(ack.issuer_role_id, |role| role.id);
        let origin = (origin_role_id, ack.executor_entity_id);
        let recipient = ack.recipient_entity_id;
        let Some(message_id) = transport::transmit_c2_message_from_entity(
            game,
            origin,
            recipient,
            ACK_PROFILE,
            format!("Execution acknowledgement for order {intent_id}"),
            fields,
        ) else {
            if game.status != GameStatus::Running {
                return;
            }
            continue;
        };
        await_message_delivery(
            game,
            message_id,
            DeliveryAction::ReceiveExecutionAck { intent_id },
        );
    }
}

pub(super) fn receive(game: &mut Game, message_id: Uuid, intent_id: Uuid) {
    let Some(record) = game
        .network_messages
        .iter()
        .find(|record| record.message.id == message_id)
    else {
        return;
    };
    let Some(ack) = game.execution_acks.results.get_mut(&intent_id) else {
        return;
    };
    if record.state == MessageState::Delivered
        && record.message.profile_id == ACK_PROFILE
        && record.message.header.origin_entity_id == ack.executor_entity_id
        && record.message.header.recipient_entity_id == ack.recipient_entity_id
        && record.message.fields.get("intent_id") == Some(&serde_json::json!(intent_id))
        && record.message.fields.get("executed_tick") == Some(&serde_json::json!(ack.executed_tick))
        && record.message.fields.get("result") == Some(&serde_json::json!(ack.result))
    {
        if let Some(at) = record.delivered_at_ns {
            ack.received_tick.get_or_insert(at.div_ceil(1_000_000_000));
        }
    }
}

pub(super) fn confirmation(game: &Game, intent_id: Uuid) -> Option<(u64, u64, &OrderResult)> {
    let ack = game.execution_acks.results.get(&intent_id)?;
    Some((ack.executed_tick, ack.received_tick?, &ack.result))
}

/// An unanswered command becomes unconfirmed on a clock the issuer can observe.
pub(super) fn confirmation_timeout(game: &Game) -> u64 {
    game.message_profiles
        .get(ACK_PROFILE)
        .map_or(120, |profile| profile.expiry_ticks)
        .saturating_mul(u64::from(MAX_ATTEMPTS))
        .saturating_add(RETRY_TICKS.saturating_mul(u64::from(MAX_ATTEMPTS - 1)))
}

#[cfg(test)]
mod tests;
