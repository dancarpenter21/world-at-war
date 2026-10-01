use super::*;
use crate::authority_tests::{game, game_from_scenario};
use sim_core::{OrderKind, OrderStatus};

const MOVE_PROFILE: &str = "public-safe.jseries.move-order.v1";

fn move_order(game: &mut Game) -> Uuid {
    let role = Uuid::from_u128(106);
    let target = Uuid::from_u128(11);
    let outcome = submit_authority_action(
        game,
        role,
        "move".into(),
        target,
        "Move north".into(),
        Some(PlayerIntent {
            intent_id: Uuid::new_v4(),
            issuer_role: role,
            target,
            kind: OrderKind::Move {
                north_mps: 10.0,
                east_mps: 0.0,
            },
            requested_tick: game.simulation.tick(),
        }),
    )
    .unwrap();
    match outcome {
        SubmissionOutcome::Queued { message_id, .. } => message_id,
        _ => panic!("expected a direct order"),
    }
}

fn record(game: &Game, message_id: Uuid) -> &NetworkMessageRecord {
    game.network_messages
        .iter()
        .find(|record| record.message.id == message_id)
        .unwrap()
}

struct EventFile(PathBuf);
impl EventFile {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("world-at-war-events-{}.jsonl", Uuid::new_v4()));
        std::fs::write(&path, []).unwrap();
        Self(path)
    }
}
impl Drop for EventFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn networked_order_waits_for_delivery_before_execution_and_executes_once() {
    let mut game = game();
    let message_id = move_order(&mut game);
    assert_eq!(record(&game, message_id).state, MessageState::Queued);
    assert!(record(&game, message_id).delivered_at_ns.is_none());
    process_network_messages(&mut game);
    assert_eq!(record(&game, message_id).state, MessageState::InTransit);
    assert_eq!(record(&game, message_id).started_at_ns, Some(0));
    assert!(game.simulation.drain_order_results().is_empty());
    advance_game_tick(&mut game);
    assert_eq!(record(&game, message_id).state, MessageState::Delivered);
    assert!(record(&game, message_id)
        .delivered_at_ns
        .is_some_and(|time| time <= 1_000_000_000));
    assert!(game.simulation.drain_order_results().is_empty());
    advance_game_tick(&mut game);
    let results = game.simulation.drain_order_results();
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0].status, OrderStatus::Accepted));
    advance_game_tick(&mut game);
    assert!(game.simulation.drain_order_results().is_empty());
    assert!(game.packet_messages.is_empty());
    assert!(game.pending_deliveries.is_empty());
}

#[test]
fn two_orders_at_one_tick_share_the_queue_without_advancing_time_into_the_future() {
    let mut game = game();
    let first = move_order(&mut game);
    let second = move_order(&mut game);
    assert_eq!(game.simulation.tick(), 0);
    assert_eq!(record(&game, first).state, MessageState::Queued);
    assert_eq!(record(&game, second).state, MessageState::Queued);
    process_network_messages(&mut game);
    assert_eq!(record(&game, first).state, MessageState::InTransit);
    assert_eq!(record(&game, second).state, MessageState::Queued);
    let projection = game
        .simulation
        .projection_for(Uuid::from_u128(5), Side::Blue);
    assert!(projection
        .communication_links
        .iter()
        .any(|link| link.queued_packets == 1 && link.queued_bytes > 0));
    advance_game_tick(&mut game);
    assert_eq!(record(&game, first).state, MessageState::Delivered);
    assert_eq!(record(&game, second).state, MessageState::Delivered);
    assert!(record(&game, first).delivered_at_ns < record(&game, second).delivered_at_ns);
    advance_game_tick(&mut game);
    assert_eq!(game.simulation.drain_order_results().len(), 2);
}

#[test]
fn frozen_message_profile_controls_packet_priority_hop_budget_and_expiry() {
    let mut game = game();
    let profile = game.message_profiles.get_mut(MOVE_PROFILE).unwrap();
    profile.default_priority = 191;
    profile.ttl_hops = 7;
    profile.expiry_ticks = 13;
    let message_id = move_order(&mut game);
    let events = game.simulation.advance_network().unwrap();
    let packet = events
        .iter()
        .find_map(|event| match event {
            NetworkEvent::TransmissionStarted { packet, .. } => Some(packet),
            _ => None,
        })
        .unwrap();
    assert_eq!(record(&game, message_id).message.header.priority, 191);
    assert_eq!(packet.metadata().priority, 191);
    assert_eq!(packet.hops_remaining(), 7);
    assert_eq!(
        packet.metadata().expires_at,
        Some(NetworkTime::from_nanos(13_000_000_000))
    );
    assert_eq!(packet.metadata().traffic_class, 2);
}

#[test]
fn dropped_command_never_reaches_the_intent_executor() {
    let mut scenario = global_crisis_scenario();
    for channel in &mut scenario.network.channels {
        channel.state = sim_core::CommunicationChannelState::Severed;
    }
    let mut game = game_from_scenario(scenario);
    let message_id = move_order(&mut game);
    process_network_messages(&mut game);
    assert_eq!(record(&game, message_id).state, MessageState::Dropped);
    assert!(record(&game, message_id)
        .drop_reason
        .as_ref()
        .is_some_and(|reason| reason.contains("ChannelSevered")));
    for _ in 0..3 {
        advance_game_tick(&mut game);
    }
    assert!(game.simulation.drain_order_results().is_empty());
    assert!(game.pending_deliveries.is_empty());
}

#[test]
fn expired_command_never_executes_after_a_slow_delivery() {
    let mut scenario = global_crisis_scenario();
    for channel in &mut scenario.network.channels {
        channel.bit_rate_bps = 1_000;
    }
    let mut game = game_from_scenario(scenario);
    game.message_profiles
        .get_mut(MOVE_PROFILE)
        .unwrap()
        .expiry_ticks = 1;
    let message_id = move_order(&mut game);
    for _ in 0..10 {
        advance_game_tick(&mut game);
    }
    assert_eq!(record(&game, message_id).state, MessageState::Expired);
    assert_eq!(
        record(&game, message_id).drop_reason.as_deref(),
        Some("Expired")
    );
    assert!(record(&game, message_id).delivered_at_ns.is_none());
    assert!(game.simulation.drain_order_results().is_empty());
}

#[test]
fn pause_preserves_pending_wire_and_order_state_until_resume() {
    let mut game = game();
    let message_id = move_order(&mut game);
    game.status = GameStatus::Paused;
    for _ in 0..4 {
        advance_game_tick(&mut game);
    }
    assert_eq!(game.simulation.tick(), 0);
    assert_eq!(record(&game, message_id).state, MessageState::Queued);
    assert!(game.simulation.drain_order_results().is_empty());
    game.status = GameStatus::Running;
    advance_game_tick(&mut game);
    assert_eq!(record(&game, message_id).state, MessageState::Delivered);
    advance_game_tick(&mut game);
    assert_eq!(game.simulation.drain_order_results().len(), 1);
}

#[test]
fn lifecycle_history_is_append_only_while_projection_has_one_current_message() {
    let file = EventFile::new();
    let mut game = game();
    game.network_event_path = Some(file.0.clone());
    let message_id = move_order(&mut game);
    advance_game_tick(&mut game);
    let persisted = std::fs::read_to_string(&file.0).unwrap();
    let events: Vec<serde_json::Value> = persisted
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .map(|event| event["state"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["queued", "in_transit", "delivered"]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event["sequence"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(game.network_messages.len(), 1);
    assert_eq!(game.network_message_events.len(), 3);
    assert_eq!(record(&game, message_id).sequence, 3);
    assert_eq!(game.network_message_events[0].state, MessageState::Queued);
    assert_eq!(
        game.network_message_events[1].state,
        MessageState::InTransit
    );
}

#[test]
fn event_store_failure_during_delivery_pauses_and_discards_every_pending_action() {
    let file = EventFile::new();
    let mut game = game();
    game.network_event_path = Some(file.0.clone());
    let first = move_order(&mut game);
    let second = move_order(&mut game);
    process_network_messages(&mut game);
    game.network_event_path = Some(
        std::env::temp_dir()
            .join(format!("missing-{}", Uuid::new_v4()))
            .join("events.jsonl"),
    );
    advance_game_tick(&mut game);
    assert_eq!(game.status, GameStatus::Paused);
    assert_eq!(record(&game, first).state, MessageState::Dropped);
    assert_eq!(record(&game, second).state, MessageState::Dropped);
    assert!(game.packet_messages.is_empty());
    assert!(game.pending_deliveries.is_empty());
    assert!(game.simulation.drain_order_results().is_empty());
    game.network_event_path = None;
    game.status = GameStatus::Running;
    for _ in 0..3 {
        advance_game_tick(&mut game);
    }
    assert!(game.simulation.drain_order_results().is_empty());
}

fn approval_scenario() -> Scenario {
    let mut scenario = global_crisis_scenario();
    let policy = scenario
        .authority
        .policies
        .iter_mut()
        .find(|policy| policy.id == Uuid::from_u128(302))
        .unwrap();
    policy.direct_role_ids.clear();
    policy.request_role_ids = vec![Uuid::from_u128(106)];
    policy.decision_steps = vec![sim_core::AuthorityDecisionStep {
        role_id: Uuid::from_u128(112),
        vacant_delay_ticks: 1,
        approve_probability_bps: 10_000,
    }];
    scenario
}

fn request_move(game: &mut Game) -> Uuid {
    let role = Uuid::from_u128(106);
    let target = Uuid::from_u128(11);
    let outcome = submit_authority_action(
        game,
        role,
        "move".into(),
        target,
        "Approve move".into(),
        Some(PlayerIntent {
            intent_id: Uuid::new_v4(),
            issuer_role: role,
            target,
            kind: OrderKind::Move {
                north_mps: 10.0,
                east_mps: 0.0,
            },
            requested_tick: game.simulation.tick(),
        }),
    )
    .unwrap();
    match outcome {
        SubmissionOutcome::PendingAuthority { request_id, .. } => request_id,
        _ => panic!("expected authority request"),
    }
}

#[test]
fn approval_and_execution_wait_for_both_network_handoffs() {
    let mut game = game_from_scenario(approval_scenario());
    let request_id = request_move(&mut game);
    assert!(matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::InTransit { .. }
    ));
    advance_game_tick(&mut game);
    assert!(matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::WaitingVacant {
            resolves_at_tick: 2,
            ..
        }
    ));
    assert!(game.authority_requests[&request_id].decisions.is_empty());
    advance_game_tick(&mut game);
    assert_eq!(game.authority_requests[&request_id].decisions.len(), 1);
    assert!(matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::InTransit { .. }
    ));
    assert!(game.simulation.drain_order_results().is_empty());
    advance_game_tick(&mut game);
    assert!(matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::Approved
    ));
    assert!(game.simulation.drain_order_results().is_empty());
    advance_game_tick(&mut game);
    let results = game.simulation.drain_order_results();
    assert_eq!(results.len(), 1);
    assert!(matches!(results[0].status, OrderStatus::Accepted));
}

#[test]
fn a_dropped_authority_request_never_starts_the_vacant_timer() {
    let mut scenario = approval_scenario();
    for channel in &mut scenario.network.channels {
        channel.state = sim_core::CommunicationChannelState::Severed;
    }
    let mut game = game_from_scenario(scenario);
    let request_id = request_move(&mut game);
    for _ in 0..5 {
        advance_game_tick(&mut game);
    }
    assert!(matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::BlockedComms
    ));
    assert!(game.authority_requests[&request_id].decisions.is_empty());
    assert!(game.simulation.drain_order_results().is_empty());
}

#[test]
fn approval_cannot_execute_when_the_final_command_link_is_severed() {
    let mut scenario = approval_scenario();
    let target = Uuid::from_u128(11);
    let origin = Uuid::from_u128(10);
    let link = scenario
        .communication_links
        .iter()
        .find(|link| link.from_entity_id == origin && link.to_entity_id == target)
        .unwrap();
    scenario
        .network
        .channels
        .iter_mut()
        .find(|channel| channel.id == link.channel_id)
        .unwrap()
        .state = sim_core::CommunicationChannelState::Severed;
    let mut game = game_from_scenario(scenario);
    let request_id = request_move(&mut game);
    for _ in 0..5 {
        advance_game_tick(&mut game);
    }
    assert_eq!(game.authority_requests[&request_id].decisions.len(), 1);
    assert!(matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::BlockedComms
    ));
    assert!(game.simulation.drain_order_results().is_empty());
}
