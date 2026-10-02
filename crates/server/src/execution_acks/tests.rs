use super::*;
use crate::authority_tests::{game, game_from_scenario};
use sim_core::{CommunicationChannelState, OrderKind, OrderStatus};

const ROLE: Uuid = Uuid::from_u128(106);
const PLAYER: Uuid = Uuid::from_u128(8_101);
const TARGET: Uuid = Uuid::from_u128(11);
const INTENT: Uuid = Uuid::from_u128(8_102);

fn hold(game: &mut Game, role: Uuid) {
    let role = game.roles.get_mut(&role).unwrap();
    role.owner = Some(PLAYER);
    role.lease_generation = 4;
}

fn submit_move(game: &mut Game) -> PlayerIntent {
    let intent = PlayerIntent {
        intent_id: INTENT,
        issuer_role: ROLE,
        target: TARGET,
        kind: OrderKind::Move {
            north_mps: 12.0,
            east_mps: 34.0,
        },
        requested_tick: 1,
    };
    submit(game, intent.clone());
    intent
}

fn submit(game: &mut Game, intent: PlayerIntent) {
    intents::submit_player_intent(
        game,
        intent.issuer_role,
        SubmitIntentRequest {
            player_id: PLAYER,
            lease_generation: 4,
            intent,
        },
    )
    .unwrap();
}

fn receipt(game: &Game, role: Uuid, intent: Uuid) -> serde_json::Value {
    serde_json::to_value(
        intents::receipt_for(
            game,
            role,
            intent,
            &serde_json::from_value(serde_json::json!({
                "player_id": PLAYER, "lease_generation": 4,
            }))
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn step(game: &mut Game, count: usize) {
    for _ in 0..count {
        advance_game_tick(game);
    }
}

fn reply_channels(scenario: &Scenario) -> BTreeSet<String> {
    let terminal = scenario
        .authority
        .roles
        .iter()
        .find(|role| role.id == ROLE)
        .unwrap()
        .location_unit_id;
    scenario
        .communication_links
        .iter()
        .filter(|link| link.from_entity_id == TARGET && link.to_entity_id == terminal)
        .map(|link| link.channel_id.to_string())
        .collect()
}

fn acks(game: &Game) -> Vec<&NetworkMessageRecord> {
    game.network_messages
        .iter()
        .filter(|record| record.message.profile_id == ACK_PROFILE)
        .collect()
}

#[test]
fn execution_stays_unknown_until_the_return_packet_arrives_and_is_role_scoped() {
    let mut game = game();
    hold(&mut game, ROLE);
    submit_move(&mut game);
    step(&mut game, 1);
    let before_execution = receipt(&game, ROLE, INTENT);
    assert_eq!(before_execution["state"], "awaiting_acknowledgement");
    step(&mut game, 1);
    assert_eq!(receipt(&game, ROLE, INTENT), before_execution);
    assert_eq!(
        game.simulation
            .projection_for(TARGET, Side::Blue)
            .own_units
            .iter()
            .find(|unit| unit.id == TARGET)
            .unwrap()
            .velocity
            .north_mps,
        12.0
    );
    let reply = acks(&game)[0];
    assert_eq!(reply.message.header.origin_entity_id, TARGET);
    assert_eq!(
        reply.message.header.recipient_entity_id,
        game.roles[&ROLE].location_unit_id
    );
    assert!(!network_message_visible(reply, &game.roles[&ROLE]));
    let pilot = game
        .roles
        .values()
        .find(|role| role.location_unit_id == TARGET)
        .unwrap();
    assert!(network_message_visible(reply, pilot));
    let unrelated = game
        .roles
        .values()
        .find(|role| role.id != ROLE && role.location_unit_id != TARGET)
        .unwrap();
    assert!(!network_message_visible(reply, unrelated));
    step(&mut game, 1);
    let confirmed = receipt(&game, ROLE, INTENT);
    assert_eq!(confirmed["state"], "executed");
    assert_eq!(confirmed["executed_tick"], 2);
    assert_eq!(confirmed["acknowledged_tick"], 3);
    assert!(network_message_visible(acks(&game)[0], &game.roles[&ROLE]));
    let message_id = acks(&game)[0].message.id;
    receive(&mut game, message_id, INTENT);
    step(&mut game, 15);
    assert_eq!(receipt(&game, ROLE, INTENT), confirmed);
    assert_eq!(acks(&game).len(), 1);
}

#[test]
fn severed_return_path_retries_four_times_without_reexecuting_or_revealing_the_result() {
    let mut scenario = global_crisis_scenario();
    let reverse = reply_channels(&scenario);
    assert!(!reverse.is_empty());
    for channel in &mut scenario.network.channels {
        if reverse.contains(&channel.id.to_string()) {
            channel.state = CommunicationChannelState::Severed;
        }
    }
    let mut game = game_from_scenario(scenario);
    hold(&mut game, ROLE);
    game.message_profiles
        .get_mut(ACK_PROFILE)
        .unwrap()
        .expiry_ticks = 1;
    let intent = submit_move(&mut game);
    step(&mut game, 2);
    let unknown = receipt(&game, ROLE, INTENT);
    assert_eq!(unknown["state"], "awaiting_acknowledgement");
    assert!(unknown.get("executed_tick").is_none());
    assert!(unknown.get("error").is_none());
    step(&mut game, 30);
    let replies = acks(&game);
    assert_eq!(replies.len(), 4);
    assert!(replies
        .iter()
        .all(|record| record.state == MessageState::Dropped
            && !network_message_visible(record, &game.roles[&ROLE])));
    assert_eq!(receipt(&game, ROLE, INTENT), unknown);
    assert!(settled(&game));
    submit(&mut game, intent);
    step(&mut game, 10);
    assert_eq!(acks(&game).len(), 4);
    assert!(game.simulation.drain_order_results().is_empty());
    let final_receipt = receipt(&game, ROLE, INTENT);
    assert_eq!(final_receipt["state"], "unconfirmed");
    assert!(final_receipt.get("executed_tick").is_none());
}

#[test]
fn a_timed_receiver_blackout_delays_confirmation_until_a_paced_retry_succeeds() {
    let mut scenario = global_crisis_scenario();
    let terminal = scenario
        .authority
        .roles
        .iter()
        .find(|role| role.id == ROLE)
        .unwrap()
        .location_unit_id;
    let reverse = reply_channels(&scenario);
    let center = scenario
        .units
        .iter()
        .find(|unit| unit.id == terminal)
        .unwrap()
        .position;
    scenario.jamming_regions.push(sim_core::JammingRegion {
        id: "ack-blackout".into(),
        name: "Reply blackout".into(),
        center,
        radius_m: 500.0,
        band: scenario
            .network
            .channels
            .iter()
            .filter(|channel| reverse.contains(&channel.id.to_string()))
            .find_map(|channel| channel.radio.as_ref().map(|radio| radio.band))
            .unwrap(),
        jammed: 1.0,
        active_from_tick: 0,
        active_until_tick: Some(12),
    });
    let mut game = game_from_scenario(scenario);
    hold(&mut game, ROLE);
    submit_move(&mut game);
    step(&mut game, 11);
    assert_eq!(
        receipt(&game, ROLE, INTENT)["state"],
        "awaiting_acknowledgement"
    );
    assert_eq!(acks(&game).len(), 1);
    assert_eq!(acks(&game)[0].state, MessageState::Dropped);
    step(&mut game, 3);
    let confirmed = receipt(&game, ROLE, INTENT);
    assert_eq!(confirmed["state"], "executed");
    assert_eq!(confirmed["executed_tick"], 2);
    assert!(confirmed["acknowledged_tick"].as_u64().unwrap() >= 12);
    assert_eq!(acks(&game).len(), 2);
}

#[test]
fn pause_freezes_a_return_packet_and_resume_confirms_the_original_execution_tick() {
    let mut game = game();
    hold(&mut game, ROLE);
    submit_move(&mut game);
    step(&mut game, 2);
    game.status = GameStatus::Paused;
    let frozen = receipt(&game, ROLE, INTENT);
    let events = game.network_message_events.len();
    step(&mut game, 5);
    assert_eq!(game.simulation.tick(), 2);
    assert_eq!(game.network_message_events.len(), events);
    assert_eq!(receipt(&game, ROLE, INTENT), frozen);
    game.status = GameStatus::Running;
    step(&mut game, 1);
    assert_eq!(receipt(&game, ROLE, INTENT)["executed_tick"], 2);
    assert_eq!(receipt(&game, ROLE, INTENT)["acknowledged_tick"], 3);
}

#[test]
fn expired_return_packets_never_confirm_execution_and_retries_remain_bounded() {
    let mut scenario = global_crisis_scenario();
    let reverse = reply_channels(&scenario);
    for channel in &mut scenario.network.channels {
        if reverse.contains(&channel.id.to_string()) {
            channel.bit_rate_bps = 1;
        }
    }
    let mut game = game_from_scenario(scenario);
    hold(&mut game, ROLE);
    game.message_profiles
        .get_mut(ACK_PROFILE)
        .unwrap()
        .expiry_ticks = 1;
    submit_move(&mut game);
    step(&mut game, 45);
    assert_eq!(acks(&game).len(), 4);
    assert!(acks(&game)
        .iter()
        .all(|record| record.state == MessageState::Expired));
    assert_eq!(receipt(&game, ROLE, INTENT)["state"], "unconfirmed");
    assert!(confirmation(&game, INTENT).is_none());
}

#[test]
fn failed_acknowledgement_delivery_persistence_pauses_without_granting_execution_knowledge() {
    let mut game = game();
    hold(&mut game, ROLE);
    submit_move(&mut game);
    step(&mut game, 2);
    game.network_event_path = Some(std::env::temp_dir());
    step(&mut game, 1);
    assert_eq!(game.status, GameStatus::Paused);
    assert!(game.operational_error.is_some());
    assert!(confirmation(&game, INTENT).is_none());
    assert_eq!(
        receipt(&game, ROLE, INTENT)["state"],
        "awaiting_acknowledgement"
    );
    assert!(!network_message_visible(acks(&game)[0], &game.roles[&ROLE]));
}

#[test]
fn local_execution_is_confirmed_only_after_its_acknowledgement_is_persisted() {
    for persistence_failure in [false, true] {
        let mut game = game();
        hold(&mut game, ROLE);
        game.roles.get_mut(&ROLE).unwrap().location_unit_id = TARGET;
        submit_move(&mut game);
        if persistence_failure {
            game.network_event_path = Some(std::env::temp_dir());
        }
        step(&mut game, 1);
        assert_eq!(acks(&game).len(), 1);
        assert_eq!(acks(&game)[0].packet_id, None);
        if persistence_failure {
            assert_eq!(game.status, GameStatus::Paused);
            assert!(receipt(&game, ROLE, INTENT).get("executed_tick").is_none());
        } else {
            assert_eq!(receipt(&game, ROLE, INTENT)["state"], "executed");
            assert_eq!(receipt(&game, ROLE, INTENT)["executed_tick"], 1);
            assert_eq!(receipt(&game, ROLE, INTENT)["acknowledged_tick"], 1);
        }
    }
}

#[test]
fn launch_rejection_reason_is_withheld_until_the_executor_reply_arrives() {
    let commander = Uuid::from_u128(20102);
    let mut scenario = combat_training_scenario();
    scenario
        .combat
        .as_mut()
        .unwrap()
        .units
        .iter_mut()
        .find(|unit| unit.unit_id == TARGET)
        .unwrap()
        .weapon
        .as_mut()
        .unwrap()
        .range_m = 100.0;
    let mut game = game_from_scenario(scenario);
    hold(&mut game, commander);
    let terminal = game.roles[&commander].location_unit_id;
    while game
        .simulation
        .projection_for(terminal, Side::Blue)
        .tracks
        .is_empty()
    {
        step(&mut game, 1);
        assert!(game.simulation.tick() < 15);
    }
    let track_id = game.simulation.projection_for(terminal, Side::Blue).tracks[0].track_id;
    let requested_tick = game.simulation.tick() + 1;
    submit(
        &mut game,
        PlayerIntent {
            intent_id: INTENT,
            issuer_role: commander,
            target: TARGET,
            kind: OrderKind::Engage { track_id },
            requested_tick,
        },
    );
    while game.execution_acks.results.is_empty() {
        step(&mut game, 1);
        assert!(game.simulation.tick() < 25);
    }
    assert!(matches!(
        game.execution_acks.results[&INTENT].result.status,
        OrderStatus::Rejected(_)
    ));
    let unknown = receipt(&game, commander, INTENT);
    assert_eq!(unknown["state"], "awaiting_acknowledgement");
    assert!(unknown.get("error").is_none());
    assert!(unknown.get("executed_tick").is_none());
    assert!(!network_message_visible(
        acks(&game)[0],
        &game.roles[&commander]
    ));
    for _ in 0..10 {
        step(&mut game, 1);
        if confirmation(&game, INTENT).is_some() {
            break;
        }
    }
    let known = receipt(&game, commander, INTENT);
    assert_eq!(known["state"], "rejected");
    assert!(known["error"].as_str().unwrap().contains("range"));
    assert!(
        known["acknowledged_tick"].as_u64().unwrap() > known["executed_tick"].as_u64().unwrap()
    );
    assert_eq!(
        game.simulation
            .projection_for(TARGET, Side::Blue)
            .own_units
            .iter()
            .find(|unit| unit.id == TARGET)
            .unwrap()
            .weapon
            .as_ref()
            .unwrap()
            .ammunition,
        2
    );
}

#[test]
fn an_automated_executor_without_a_local_role_cannot_expose_its_reply_in_the_issuers_outbox() {
    let mut game = game();
    hold(&mut game, ROLE);
    game.roles.retain(|_, role| role.location_unit_id != TARGET);
    submit_move(&mut game);
    step(&mut game, 2);
    let reply = acks(&game)[0];
    assert_eq!(reply.message.header.origin_role_id, ROLE);
    assert_eq!(reply.message.header.origin_entity_id, TARGET);
    assert!(!network_message_visible(reply, &game.roles[&ROLE]));
    assert!(receipt(&game, ROLE, INTENT).get("executed_tick").is_none());
    step(&mut game, 1);
    assert_eq!(receipt(&game, ROLE, INTENT)["state"], "executed");
}

#[test]
fn post_mission_radio_time_settles_final_acknowledgements_or_records_an_unconfirmed_result() {
    let commander = Uuid::from_u128(20102);
    let terminal = Uuid::from_u128(5);
    for reporting_window in [0, 20] {
        let mut scenario = combat_training_scenario();
        scenario.sensor_report_routes.clear();
        scenario.reporting_window_ticks = reporting_window;
        let reverse: BTreeSet<_> = scenario
            .communication_links
            .iter()
            .filter(|link| link.from_entity_id == TARGET && link.to_entity_id == terminal)
            .map(|link| link.channel_id.to_string())
            .collect();
        for channel in &mut scenario.network.channels {
            if reverse.contains(&channel.id.to_string()) {
                channel.bit_rate_bps = 600;
            }
        }
        let mut game = game_from_scenario(scenario);
        hold(&mut game, commander);
        step(&mut game, 1);
        let observation = game.simulation.current_sensor_tracks(TARGET)[0].clone();
        game.simulation.receive_track_report(terminal, observation);
        let track_id = game.simulation.projection_for(terminal, Side::Blue).tracks[0].track_id;
        submit(
            &mut game,
            PlayerIntent {
                intent_id: INTENT,
                issuer_role: commander,
                target: TARGET,
                kind: OrderKind::Engage { track_id },
                requested_tick: 2,
            },
        );
        while !game.simulation.mission_complete() {
            step(&mut game, 1);
            assert!(game.simulation.tick() < 25);
        }
        let combat_tick = game.simulation.tick();
        let own_role = game.roles[&commander].clone();
        let before = serde_json::to_value(intents::mission_debrief(&mut game, &own_role)).unwrap();
        assert!(before["entries"][0]["launch_tick"].is_null());
        assert!(before["entries"][0]["acknowledged_tick"].is_null());
        assert!(receipt(&game, commander, INTENT)
            .get("executed_tick")
            .is_none());
        if reporting_window == 0 {
            assert_eq!(game.status, GameStatus::Paused);
            assert_eq!(acks(&game)[0].state, MessageState::Dropped);
            assert_eq!(receipt(&game, commander, INTENT)["state"], "unconfirmed");
        } else {
            assert_eq!(game.status, GameStatus::Running);
            assert_eq!(
                receipt(&game, commander, INTENT)["state"],
                "awaiting_acknowledgement"
            );
            for _ in 0..20 {
                step(&mut game, 1);
                assert_eq!(game.simulation.tick(), combat_tick);
                if game.status == GameStatus::Paused {
                    break;
                }
            }
            assert_eq!(game.status, GameStatus::Paused);
            let confirmed = receipt(&game, commander, INTENT);
            assert_eq!(confirmed["state"], "executed");
            assert!(confirmed["acknowledged_tick"].as_u64().unwrap() > combat_tick);
            let after =
                serde_json::to_value(intents::mission_debrief(&mut game, &own_role)).unwrap();
            assert_eq!(
                after["entries"][0]["launch_tick"],
                confirmed["executed_tick"]
            );
            assert_eq!(
                after["entries"][0]["acknowledged_tick"],
                confirmed["acknowledged_tick"]
            );
            assert!(after["entries"][0]["hit"].is_null());
            assert!(after["entries"][0]["impact_tick"].is_null());
        }
    }
}
