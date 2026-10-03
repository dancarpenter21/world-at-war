use super::*;
use crate::authority_tests::{game, game_from_scenario};

const ROLE: Uuid = Uuid::from_u128(106);
const PLAYER: Uuid = Uuid::from_u128(8_001);
const TARGET: Uuid = Uuid::from_u128(11);

fn held_game() -> Game {
    let mut game = game();
    hold_role(&mut game);
    game
}

fn hold_role(game: &mut Game) {
    let role = game.roles.get_mut(&ROLE).unwrap();
    role.owner = Some(PLAYER);
    role.lease_generation = 4;
}

fn intent() -> PlayerIntent {
    PlayerIntent {
        intent_id: Uuid::from_u128(8_002),
        issuer_role: ROLE,
        target: TARGET,
        kind: OrderKind::Move {
            north_mps: 12.0,
            east_mps: 34.0,
        },
        requested_tick: 1,
    }
}

fn submit(game: &mut Game, intent: PlayerIntent) -> SubmissionResult {
    submit_player_intent(
        game,
        ROLE,
        SubmitIntentRequest {
            player_id: PLAYER,
            lease_generation: 4,
            intent,
        },
    )
}

fn receipt(game: &Game) -> IntentReceipt {
    receipt_for(
        game,
        ROLE,
        intent().intent_id,
        &ReceiptQuery {
            player_id: PLAYER,
            lease_generation: 4,
        },
    )
    .unwrap()
}

fn tick(game: &mut Game) {
    advance_game_tick(game);
    record_execution_results(game);
}

#[test]
fn duplicate_submission_sends_one_packet_and_executes_once() {
    let mut game = held_game();
    let first = submit(&mut game, intent()).unwrap();
    assert_eq!(submit(&mut game, intent()).unwrap(), first);
    assert_eq!(game.network_messages.len(), 1);
    assert_eq!(game.pending_deliveries.len(), 1);
    assert_eq!(receipt(&game).state, IntentState::Queued);
    process_network_messages(&mut game);
    assert_eq!(receipt(&game).state, IntentState::InTransit);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::AwaitingAcknowledgement);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::AwaitingAcknowledgement);
    assert!(receipt(&game).executed_tick.is_none());
    assert_eq!(game.network_messages.len(), 2);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::Executed);
    assert_eq!(receipt(&game).executed_tick, Some(2));
    assert_eq!(receipt(&game).acknowledged_tick, Some(3));
    assert_eq!(submit(&mut game, intent()).unwrap(), first);
    tick(&mut game);
    assert_eq!(receipt(&game).executed_tick, Some(2));
    assert_eq!(game.network_messages.len(), 2);
    assert!(game.simulation.drain_order_results().is_empty());
    let projection = game.simulation.projection_for(TARGET, Side::Blue);
    let unit = projection
        .own_units
        .iter()
        .find(|unit| unit.id == TARGET)
        .unwrap();
    assert_eq!(unit.velocity.north_mps, 12.0);
    assert_eq!(unit.velocity.east_mps, 34.0);
}

#[test]
fn reused_order_id_cannot_change_command_or_send_another_packet() {
    let mut game = held_game();
    submit(&mut game, intent()).unwrap();
    let mut changed = intent();
    changed.kind = OrderKind::Move {
        north_mps: 0.0,
        east_mps: 100.0,
    };
    let (status, Json(error)) = submit(&mut game, changed).unwrap_err();
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.code, "intent_conflict");
    assert_eq!(game.network_messages.len(), 1);
}

#[test]
fn accepted_submission_can_be_recovered_while_paused_but_new_orders_cannot() {
    let mut game = held_game();
    let first = submit(&mut game, intent()).unwrap();
    game.status = GameStatus::Paused;
    assert_eq!(submit(&mut game, intent()).unwrap(), first);
    let mut new = intent();
    new.intent_id = Uuid::from_u128(8_003);
    assert_eq!(
        submit(&mut game, new).unwrap_err().1 .0.code,
        "game_not_running"
    );
    assert_eq!(game.network_messages.len(), 1);
}

#[test]
fn former_role_holder_cannot_replay_or_read_a_receipt() {
    let mut game = held_game();
    submit(&mut game, intent()).unwrap();
    let role = game.roles.get_mut(&ROLE).unwrap();
    role.owner = Some(Uuid::from_u128(8_004));
    role.lease_generation = 5;
    assert_eq!(
        submit(&mut game, intent()).unwrap_err().1 .0.code,
        "invalid_role_lease"
    );
    assert!(receipt_for(
        &game,
        ROLE,
        intent().intent_id,
        &ReceiptQuery {
            player_id: PLAYER,
            lease_generation: 4
        }
    )
    .is_err());
    let result = receipt_for(
        &game,
        ROLE,
        intent().intent_id,
        &ReceiptQuery {
            player_id: Uuid::from_u128(8_004),
            lease_generation: 5,
        },
    );
    assert!(matches!(result, Err((StatusCode::NOT_FOUND, _))));
}

#[test]
fn failed_submission_is_cached_without_repeating_transport_side_effects() {
    let mut scenario = global_crisis_scenario();
    scenario
        .communication_links
        .retain(|link| link.to_entity_id != TARGET);
    let mut game = game_from_scenario(scenario);
    hold_role(&mut game);
    let first = submit(&mut game, intent()).unwrap_err();
    assert_eq!(first.1 .0.code, "blocked_comms");
    let count = game.network_message_events.len();
    let second = submit(&mut game, intent()).unwrap_err();
    assert_eq!(second.1 .0.error, first.1 .0.error);
    assert_eq!(game.network_message_events.len(), count);
    assert_eq!(receipt(&game).state, IntentState::Dropped);
    assert!(receipt(&game).message_id.is_some());
}

#[test]
fn receipt_reports_a_channel_drop_without_claiming_execution() {
    let mut scenario = global_crisis_scenario();
    for channel in &mut scenario.network.channels {
        channel.state = sim_core::CommunicationChannelState::Severed;
    }
    let mut game = game_from_scenario(scenario);
    hold_role(&mut game);
    submit(&mut game, intent()).unwrap();
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::Dropped);
    assert!(receipt(&game).error.is_some());
    assert!(receipt(&game).executed_tick.is_none());
}

#[test]
fn authority_receipt_tracks_approval_and_final_command_execution() {
    let mut scenario = global_crisis_scenario();
    let policy = scenario
        .authority
        .policies
        .iter_mut()
        .find(|policy| policy.id == Uuid::from_u128(302))
        .unwrap();
    policy.direct_role_ids.clear();
    policy.request_role_ids = vec![ROLE];
    policy.decision_steps = vec![sim_core::AuthorityDecisionStep {
        role_id: Uuid::from_u128(112),
        vacant_delay_ticks: 1,
        approve_probability_bps: 10_000,
    }];
    let mut game = game_from_scenario(scenario);
    hold_role(&mut game);
    assert!(matches!(
        submit(&mut game, intent()).unwrap(),
        SubmissionOutcome::PendingAuthority { .. }
    ));
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::AwaitingAuthority);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::InTransit);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::AwaitingAcknowledgement);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::AwaitingAcknowledgement);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::Executed);
    assert!(receipt(&game).request_id.is_some());
}

#[test]
fn command_packet_contains_the_vectors_and_original_order_identity() {
    let mut game = held_game();
    submit(&mut game, intent()).unwrap();
    let record = &game.network_messages[0];
    assert_eq!(
        record.message.fields["intent_id"],
        intent().intent_id.to_string()
    );
    assert_eq!(record.message.fields["north_mps"], serde_json::json!(12.0));
    assert_eq!(record.message.fields["east_mps"], serde_json::json!(34.0));
    let encoded: C2Message = serde_json::from_slice(&record.encoded_bytes).unwrap();
    assert_eq!(encoded.fields, record.message.fields);
}

#[test]
fn nonfinite_and_excessive_movement_is_rejected_before_transmission() {
    for (north_mps, east_mps) in [(f64::NAN, 0.0), (0.0, f64::INFINITY), (800.0, 800.0)] {
        let mut game = held_game();
        let mut command = intent();
        command.kind = OrderKind::Move {
            north_mps,
            east_mps,
        };
        assert_eq!(
            submit(&mut game, command).unwrap_err().1 .0.code,
            "invalid_movement"
        );
        assert!(game.network_messages.is_empty());
        assert!(game.intent_submissions.is_empty());
    }
}

const COMBAT_COMMANDER: Uuid = Uuid::from_u128(20102);
const COMBAT_PILOT: Uuid = Uuid::from_u128(20103);

fn combat_game() -> Game {
    let mut game = game_from_scenario(sim_scenario::combat_training_scenario());
    for role_id in [COMBAT_COMMANDER, COMBAT_PILOT] {
        let role = game.roles.get_mut(&role_id).unwrap();
        role.owner = Some(PLAYER);
        role.lease_generation = 4;
    }
    game
}

fn combat_ammunition(game: &mut Game) -> u32 {
    game.simulation
        .projection_for(Uuid::from_u128(5), Side::Blue)
        .own_units
        .iter()
        .find(|unit| unit.id == TARGET)
        .unwrap()
        .weapon
        .as_ref()
        .unwrap()
        .ammunition
}

fn engagement(game: &mut Game, role_id: Uuid, id: u128) -> PlayerIntent {
    let terminal = game.roles[&role_id].location_unit_id;
    let track = game.simulation.projection_for(terminal, Side::Blue).tracks[0].track_id;
    PlayerIntent {
        intent_id: Uuid::from_u128(id),
        issuer_role: role_id,
        target: TARGET,
        kind: OrderKind::Engage { track_id: track },
        requested_tick: game.simulation.tick() + 1,
    }
}

fn submit_engagement(game: &mut Game, intent: PlayerIntent) -> SubmissionResult {
    submit_player_intent(
        game,
        intent.issuer_role,
        SubmitIntentRequest {
            player_id: PLAYER,
            lease_generation: 4,
            intent,
        },
    )
}

fn wait_for_commanders_report(game: &mut Game) {
    for _ in 0..10 {
        tick(game);
        if !game
            .simulation
            .projection_for(Uuid::from_u128(5), Side::Blue)
            .tracks
            .is_empty()
        {
            return;
        }
    }
    panic!("expected a radio-delivered commander report");
}

#[test]
fn engagement_waits_for_radio_delivery_consumes_one_round_and_ends_the_exercise_once() {
    let mut game = combat_game();
    wait_for_commanders_report(&mut game);
    let intent = engagement(&mut game, COMBAT_COMMANDER, 91001);
    let original = submit_engagement(&mut game, intent.clone()).unwrap();
    assert_eq!(
        submit_engagement(&mut game, intent.clone()).unwrap(),
        original
    );
    let SubmissionOutcome::Queued { message_id, .. } = original else {
        panic!("expected a remote engagement");
    };
    let record = game
        .network_messages
        .iter()
        .find(|record| record.message.id == message_id)
        .unwrap();
    assert!(record.message.fields.contains_key("aim_point"));
    assert!(record.message.fields.contains_key("observed_tick"));
    assert_eq!(combat_ammunition(&mut game), 2);
    let mut delivered = false;
    for _ in 0..12 {
        tick(&mut game);
        let record = game
            .network_messages
            .iter()
            .find(|record| record.message.id == message_id)
            .unwrap();
        if record.state == MessageState::Delivered {
            delivered = true;
            break;
        }
        assert_eq!(combat_ammunition(&mut game), 2);
    }
    assert!(delivered);
    assert_eq!(combat_ammunition(&mut game), 2);
    tick(&mut game);
    assert_eq!(combat_ammunition(&mut game), 1);
    let receipt = receipt_for(
        &game,
        COMBAT_COMMANDER,
        intent.intent_id,
        &ReceiptQuery {
            player_id: PLAYER,
            lease_generation: 4,
        },
    )
    .unwrap();
    assert_eq!(receipt.state, IntentState::AwaitingAcknowledgement);
    assert!(receipt.executed_tick.is_none());
    for _ in 0..12 {
        tick(&mut game);
        if game.simulation.mission_complete() {
            break;
        }
    }
    assert!(game.simulation.mission_complete());
    assert_eq!(game.status, GameStatus::Paused);
    let confirmed = receipt_for(
        &game,
        COMBAT_COMMANDER,
        intent.intent_id,
        &ReceiptQuery {
            player_id: PLAYER,
            lease_generation: 4,
        },
    )
    .unwrap();
    assert_eq!(confirmed.state, IntentState::Executed);
    assert!(confirmed.acknowledged_tick.unwrap() > confirmed.executed_tick.unwrap());
    let finished_tick = game.simulation.tick();
    for _ in 0..4 {
        tick(&mut game);
    }
    assert_eq!(game.simulation.tick(), finished_tick);
    assert_eq!(submit_engagement(&mut game, intent).unwrap(), original);
    assert_eq!(combat_ammunition(&mut game), 1);
    assert_eq!(
        game.network_messages
            .iter()
            .filter(|record| record.message.profile_id == "public-safe.jseries.engage-order.v1")
            .count(),
        1
    );
}

#[test]
fn unreceived_and_raw_enemy_identifiers_cannot_bypass_the_commanders_knowledge() {
    let mut game = combat_game();
    tick(&mut game);
    let local_track = game.simulation.current_sensor_tracks(TARGET)[0].track_id;
    for track_id in [local_track, Uuid::from_u128(51)] {
        let error = submit_engagement(
            &mut game,
            PlayerIntent {
                intent_id: Uuid::new_v4(),
                issuer_role: COMBAT_COMMANDER,
                target: TARGET,
                kind: OrderKind::Engage { track_id },
                requested_tick: 2,
            },
        )
        .unwrap_err();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.1 .0.code, "invalid_engagement");
    }
    assert_eq!(combat_ammunition(&mut game), 2);
    assert!(!game
        .network_messages
        .iter()
        .any(|record| record.message.profile_id == "public-safe.jseries.engage-order.v1"));
}

#[test]
fn an_expired_engagement_never_launches_or_consumes_a_round() {
    let mut game = combat_game();
    wait_for_commanders_report(&mut game);
    game.message_profiles
        .get_mut("public-safe.jseries.engage-order.v1")
        .unwrap()
        .expiry_ticks = 1;
    let intent = engagement(&mut game, COMBAT_COMMANDER, 91002);
    submit_engagement(&mut game, intent.clone()).unwrap();
    for _ in 0..12 {
        tick(&mut game);
    }
    assert_eq!(combat_ammunition(&mut game), 2);
    assert_eq!(
        receipt_for(
            &game,
            COMBAT_COMMANDER,
            intent.intent_id,
            &ReceiptQuery {
                player_id: PLAYER,
                lease_generation: 4
            }
        )
        .unwrap()
        .state,
        IntentState::Expired
    );
    assert!(!game.simulation.mission_complete());
}

#[test]
fn a_pilots_shot_waits_for_human_authority_and_the_approved_delivery_or_denial() {
    for approved in [false, true] {
        let mut game = combat_game();
        tick(&mut game);
        let intent = engagement(&mut game, COMBAT_PILOT, 91003);
        let SubmissionOutcome::PendingAuthority { request_id, .. } =
            submit_engagement(&mut game, intent).unwrap()
        else {
            panic!("expected approval workflow");
        };
        for _ in 0..12 {
            tick(&mut game);
            if matches!(
                game.authority_requests[&request_id].status,
                AuthorityRequestStatus::PendingHuman { .. }
            ) {
                break;
            }
        }
        assert!(matches!(
            game.authority_requests[&request_id].status,
            AuthorityRequestStatus::PendingHuman { .. }
        ));
        assert_eq!(combat_ammunition(&mut game), 2);
        let mut request = game.authority_requests.remove(&request_id).unwrap();
        advance_authority_request(&mut game, &mut request, approved, false);
        game.authority_requests.insert(request_id, request);
        assert_eq!(combat_ammunition(&mut game), 2);
        for _ in 0..12 {
            tick(&mut game);
        }
        assert_eq!(combat_ammunition(&mut game), if approved { 1 } else { 2 });
        assert_eq!(game.simulation.mission_complete(), approved);
    }
}

#[test]
fn combat_time_limit_pauses_the_server_clock_and_preserves_the_failure_outcome() {
    let mut game = combat_game();
    for _ in 0..100 {
        tick(&mut game);
    }
    assert_eq!(game.simulation.tick(), 90);
    assert_eq!(game.status, GameStatus::Paused);
    let mission = game
        .simulation
        .projection_for(Uuid::from_u128(5), Side::Blue)
        .combat
        .unwrap()
        .mission
        .unwrap();
    assert_eq!(mission.status, sim_core::combat::MissionStatus::Failed);
    assert_eq!(mission.finished_tick, Some(90));
    assert_eq!(combat_ammunition(&mut game), 2);
}

#[test]
fn a_delivered_order_scheduled_after_the_mission_deadline_has_no_execution_confirmation() {
    let mut game = combat_game();
    let intent = PlayerIntent {
        intent_id: Uuid::from_u128(91004),
        issuer_role: COMBAT_COMMANDER,
        target: TARGET,
        kind: OrderKind::Move {
            north_mps: 10.0,
            east_mps: 0.0,
        },
        requested_tick: 100,
    };
    submit_engagement(&mut game, intent.clone()).unwrap();
    for _ in 0..90 {
        tick(&mut game);
    }
    let receipt = receipt_for(
        &game,
        COMBAT_COMMANDER,
        intent.intent_id,
        &ReceiptQuery {
            player_id: PLAYER,
            lease_generation: 4,
        },
    )
    .unwrap();
    assert_eq!(receipt.state, IntentState::Unconfirmed);
    assert!(receipt.error.unwrap().contains("not been confirmed"));
    assert!(receipt.executed_tick.is_none());
}

#[test]
fn firing_aim_points_and_authority_summaries_are_hidden_from_recipients_until_delivery() {
    let mut game = combat_game();
    tick(&mut game);
    let intent = engagement(&mut game, COMBAT_PILOT, 91005);
    let SubmissionOutcome::PendingAuthority {
        request_id,
        message_id,
    } = submit_engagement(&mut game, intent).unwrap()
    else {
        panic!("expected authority request");
    };
    assert!(game.authority_requests[&request_id]
        .summary
        .contains("observed at tick"));
    assert!(authority_request_known(
        &game.authority_requests[&request_id],
        COMBAT_PILOT
    ));
    assert!(!authority_request_known(
        &game.authority_requests[&request_id],
        COMBAT_COMMANDER
    ));
    let record = game
        .network_messages
        .iter()
        .find(|record| record.message.id == message_id)
        .unwrap();
    assert!(network_message_visible(record, &game.roles[&COMBAT_PILOT]));
    assert!(!network_message_visible(
        record,
        &game.roles[&COMBAT_COMMANDER]
    ));
    let mut failed = record.clone();
    failed.state = MessageState::Dropped;
    assert!(!network_message_visible(
        &failed,
        &game.roles[&COMBAT_COMMANDER]
    ));
    let mut failed_request = game.authority_requests[&request_id].clone();
    failed_request.status = AuthorityRequestStatus::BlockedComms;
    assert!(!authority_request_known(&failed_request, COMBAT_COMMANDER));
    for _ in 0..12 {
        tick(&mut game);
        if matches!(
            game.authority_requests[&request_id].status,
            AuthorityRequestStatus::PendingHuman { .. }
        ) {
            break;
        }
    }
    assert!(authority_request_known(
        &game.authority_requests[&request_id],
        COMBAT_COMMANDER
    ));
    assert!(network_message_visible(
        game.network_messages
            .iter()
            .find(|record| record.message.id == message_id)
            .unwrap(),
        &game.roles[&COMBAT_COMMANDER]
    ));
}
