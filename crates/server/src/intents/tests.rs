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
    assert_eq!(receipt(&game).state, IntentState::AwaitingExecution);
    tick(&mut game);
    assert_eq!(receipt(&game).state, IntentState::Executed);
    assert_eq!(receipt(&game).executed_tick, Some(2));
    assert_eq!(submit(&mut game, intent()).unwrap(), first);
    tick(&mut game);
    assert_eq!(receipt(&game).executed_tick, Some(2));
    assert_eq!(game.network_messages.len(), 1);
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
    assert_eq!(receipt(&game).state, IntentState::AwaitingExecution);
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
