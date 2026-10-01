use super::*;
use sim_ai::choose_patrol_intent;
use sim_core::{Domain, OrderKind};

const RETRY_TICKS: u64 = 10;

#[derive(Default)]
pub(super) struct AiPlannerState {
    attempts: BTreeMap<Uuid, Attempt>,
}

struct Attempt {
    role_id: Uuid,
    tick: u64,
    kind: OrderKind,
}

fn order_is_pending(game: &Game, unit_id: Uuid) -> bool {
    game.pending_deliveries.values().any(|action| match action {
        DeliveryAction::ExecuteIntent(order)
        | DeliveryAction::ExecuteRequest { intent: order, .. } => order.intent.target == unit_id,
        DeliveryAction::ActivateRequest { .. } => false,
    }) || game.authority_requests.values().any(|request| {
        request.target_unit_id == unit_id
            && request.action == sim_core::ACTION_MOVE
            && matches!(
                request.status,
                AuthorityRequestStatus::InTransit { .. }
                    | AuthorityRequestStatus::PendingHuman { .. }
                    | AuthorityRequestStatus::WaitingVacant { .. }
                    | AuthorityRequestStatus::PendingExternal { .. }
            )
    })
}

pub(super) fn process_ai_orders(game: &mut Game) {
    if game.status != GameStatus::Running {
        return;
    }
    let mut roles: Vec<_> = game
        .roles
        .values()
        .filter(|role| role.ai_controlled)
        .cloned()
        .collect();
    // The smallest command scope plans first; ties use stable role IDs. An
    // aircraft receives one planner even when superior roles also control it.
    roles.sort_by_key(|role| (role.command_units.len(), role.id));
    let mut assigned = BTreeSet::new();
    for role in roles {
        let projection = game
            .simulation
            .projection_for(role.location_unit_id, role.side);
        for unit_id in role.command_units {
            if !projection
                .own_units
                .iter()
                .any(|unit| unit.id == unit_id && unit.domain == Domain::Air)
                || !assigned.insert(unit_id)
            {
                continue;
            }
            let Some(intent) = choose_patrol_intent(role.id, unit_id, &projection) else {
                continue;
            };
            if order_is_pending(game, unit_id) {
                continue;
            }
            let tick = game.simulation.tick();
            if game.ai_planner.attempts.get(&unit_id).is_some_and(|last| {
                last.role_id == role.id
                    && last.kind == intent.kind
                    && tick.saturating_sub(last.tick) < RETRY_TICKS
            }) {
                continue;
            }
            let attempt = Attempt {
                role_id: role.id,
                tick,
                kind: intent.kind.clone(),
            };
            game.ai_planner.attempts.insert(unit_id, attempt);
            let _ = submit_authority_action(
                game,
                role.id,
                intent.kind.action_key().into(),
                unit_id,
                "AI patrol".into(),
                Some(intent),
            );
            if game.status != GameStatus::Running {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
