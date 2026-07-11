//! Basic Red planner. It accepts a role projection and cannot inspect ECS truth.

use sim_core::{OrderKind, PlayerIntent, RoleProjection};
use uuid::Uuid;

pub fn choose_patrol_intent(
    role: Uuid,
    controlled_unit: Uuid,
    projection: &RoleProjection,
) -> Option<PlayerIntent> {
    let unit = projection
        .own_units
        .iter()
        .find(|unit| unit.id == controlled_unit)?;
    let east_mps = if projection.tracks.is_empty() {
        -180.0
    } else {
        -120.0
    };
    Some(PlayerIntent {
        intent_id: Uuid::new_v4(),
        issuer_role: role,
        target: unit.id,
        kind: OrderKind::Move {
            north_mps: 0.0,
            east_mps,
        },
        requested_tick: projection.tick + 1,
    })
}
