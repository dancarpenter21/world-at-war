use super::*;
use crate::authority_tests::game_from_scenario;

#[test]
fn defense_reacts_only_to_delivered_reports_and_eventually_engages() {
    let scenario = sim_scenario::ai_defense_exercise_scenario();
    scenario.validate().unwrap();
    let mut game = game_from_scenario(scenario);
    let role = Uuid::from_u128(20102);
    let initial = observation(&mut game, role, 0);
    assert!(initial.projection.tracks.is_empty());
    assert!(
        !sim_ai::agent::defensive_actions(&initial, &BTreeMap::new(), 0)
            .iter()
            .any(|a| matches!(
                a,
                AgentAction::Order {
                    kind: sim_core::OrderKind::Engage { .. },
                    ..
                }
            ))
    );
    let mut received = false;
    let mut launched = false;
    for _ in 0..200 {
        process_policies(&mut game);
        advance_game_tick(&mut game);
        intents::record_execution_results(&mut game);
        let obs = observation(&mut game, role, 0);
        received |= !obs.projection.tracks.is_empty();
        launched |= game
            .network_messages
            .iter()
            .any(|m| m.message.profile_id == "public-safe.jseries.engage-order.v1");
    }
    assert!(received, "headquarters must receive sensor reports");
    assert!(launched, "defender must issue a legal engagement");
    assert!(game
        .intent_submissions
        .values()
        .any(|s| serde_json::to_value(s).unwrap()["intent"]["kind"]
            .get("Engage")
            .is_some()));
}

#[test]
fn undetected_enemy_changes_do_not_change_role_observations_or_policy_actions() {
    let scenario = sim_scenario::ai_defense_exercise_scenario();
    let mut changed = scenario.clone();
    changed
        .units
        .iter_mut()
        .find(|u| u.side == Side::Blue)
        .unwrap()
        .position
        .longitude_deg += 30.0;
    let mut first = game_from_scenario(scenario);
    let mut second = game_from_scenario(changed);
    let role = Uuid::from_u128(20102);
    let mut a = observation(&mut first, role, 100);
    let mut b = observation(&mut second, role, 100);
    a.observation_id = Uuid::nil();
    b.observation_id = Uuid::nil();
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        serde_json::to_value(&b).unwrap()
    );
    assert_eq!(
        sim_ai::agent::defensive_actions(&a, &BTreeMap::new(), 7),
        sim_ai::agent::defensive_actions(&b, &BTreeMap::new(), 7)
    );
}

#[test]
fn policy_is_bounded_by_game_seconds_and_replays_with_identical_ids() {
    let mut game = game_from_scenario(sim_scenario::ai_defense_exercise_scenario());
    process_policies(&mut game);
    let before = recovery::encode(&game).unwrap();
    for _ in 0..60 {
        process_policies(&mut game);
    }
    assert_eq!(before, recovery::encode(&game).unwrap());
    let mut replay = recovery::decode(&before).unwrap();
    for _ in 0..25 {
        let input = recovery::TickInput::capture(&game);
        process_policies(&mut game);
        advance_game_tick(&mut game);
        intents::record_execution_results(&mut game);
        input.apply(&mut replay).unwrap();
    }
    assert_eq!(
        recovery::encode(&game).unwrap(),
        recovery::encode(&replay).unwrap()
    );
}

#[test]
fn empty_agent_state_keeps_legacy_snapshot_shape() {
    let game = game_from_scenario(sim_scenario::global_crisis_scenario());
    let json: serde_json::Value =
        serde_json::from_slice(&recovery::encode(&game).unwrap()).unwrap();
    assert!(json["game"]["ai_planner"].get("agents").is_none());
}

#[test]
fn a_thinking_external_role_reserves_units_from_overlapping_legacy_planners() {
    let mut source = game_from_scenario(sim_scenario::ai_defense_exercise_scenario());
    process_policies(&mut source);
    let mut connection = source
        .ai_planner
        .agents
        .connections
        .remove(&Uuid::from_u128(20102))
        .unwrap();
    connection.mode = Control::External;
    connection.fallback = false;
    connection.work = None;
    let mut game = game_from_scenario(sim_scenario::global_crisis_scenario());
    game.ai_planner
        .agents
        .connections
        .insert(Uuid::from_u128(202), connection);
    ai_orders::process_ai_orders(&mut game);
    assert!(
        game.network_messages.is_empty(),
        "superior legacy AI must not command externally controlled aircraft while waiting"
    );
}
