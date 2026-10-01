use super::*;
use crate::authority_tests::{game, game_from_scenario};
use sim_core::CommunicationChannelState;

fn aircraft(game: &mut Game) -> Vec<Uuid> {
    game.simulation
        .projection_for(Uuid::from_u128(50), Side::Red)
        .own_units
        .into_iter()
        .filter(|unit| unit.domain == Domain::Air)
        .map(|unit| unit.id)
        .collect()
}

#[test]
fn overlapping_ai_roles_patrol_each_aircraft_once_without_moving_headquarters() {
    let mut game = game();
    let targets = aircraft(&mut game);
    assert_eq!(targets.len(), 5);
    process_ai_orders(&mut game);
    assert_eq!(game.network_messages.len(), targets.len());
    assert!(game
        .network_messages
        .iter()
        .all(|record| record.message.header.origin_role_id == Uuid::from_u128(202)));
    process_ai_orders(&mut game);
    assert_eq!(game.network_messages.len(), targets.len());
    for _ in 0..60 {
        advance_game_tick(&mut game);
        process_ai_orders(&mut game);
    }
    assert_eq!(game.network_messages.len(), targets.len());
    assert!(game.pending_deliveries.is_empty());
    let projection = game
        .simulation
        .projection_for(Uuid::from_u128(50), Side::Red);
    for unit in projection.own_units {
        assert_eq!(
            unit.velocity.east_mps,
            if targets.contains(&unit.id) {
                -180.0
            } else {
                0.0
            }
        );
    }
    assert_eq!(game.simulation.drain_order_results().len(), targets.len());
}

#[test]
fn failed_ai_orders_retry_after_ten_ticks_instead_of_flooding_a_severed_link() {
    let mut scenario = global_crisis_scenario();
    for channel in &mut scenario.network.channels {
        channel.state = CommunicationChannelState::Severed;
    }
    let mut game = game_from_scenario(scenario);
    process_ai_orders(&mut game);
    assert_eq!(game.network_messages.len(), 5);
    for _ in 0..9 {
        advance_game_tick(&mut game);
        process_ai_orders(&mut game);
    }
    assert_eq!(game.network_messages.len(), 5);
    assert!(game
        .network_messages
        .iter()
        .all(|record| record.state == MessageState::Dropped));
    advance_game_tick(&mut game);
    process_ai_orders(&mut game);
    assert_eq!(game.network_messages.len(), 10);
    assert!(game.simulation.drain_order_results().is_empty());
}

#[test]
fn pending_slow_orders_remain_single_submissions_after_the_retry_interval() {
    let mut scenario = global_crisis_scenario();
    for channel in &mut scenario.network.channels {
        channel.bit_rate_bps = 100;
    }
    let mut game = game_from_scenario(scenario);
    process_ai_orders(&mut game);
    for _ in 0..15 {
        advance_game_tick(&mut game);
        process_ai_orders(&mut game);
    }
    assert_eq!(game.network_messages.len(), 5);
    assert_eq!(game.pending_deliveries.len(), 5);
    assert!(game.simulation.drain_order_results().is_empty());
}

#[test]
fn remote_planner_uses_reports_at_its_role_terminal_instead_of_the_target_aircraft() {
    let mut scenario = global_crisis_scenario();
    let aircraft_position = scenario
        .units
        .iter()
        .find(|unit| unit.id == Uuid::from_u128(51))
        .unwrap()
        .position;
    scenario
        .units
        .iter_mut()
        .find(|unit| unit.id == Uuid::from_u128(11))
        .unwrap()
        .position = aircraft_position;
    let mut game = game_from_scenario(scenario);
    game.simulation.step();
    assert!(!game
        .simulation
        .projection_for(Uuid::from_u128(51), Side::Red)
        .tracks
        .is_empty());
    assert!(game
        .simulation
        .projection_for(Uuid::from_u128(50), Side::Red)
        .tracks
        .is_empty());
    process_ai_orders(&mut game);
    for _ in 0..3 {
        advance_game_tick(&mut game);
    }
    let projection = game
        .simulation
        .projection_for(Uuid::from_u128(50), Side::Red);
    assert_eq!(
        projection
            .own_units
            .iter()
            .find(|unit| unit.id == Uuid::from_u128(51))
            .unwrap()
            .velocity
            .east_mps,
        -180.0
    );
}
