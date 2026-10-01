use super::*;
use crate::authority_tests::game_from_scenario;
use sim_core::{CommunicationChannelState, Track};

const SOURCE_ROLE: Uuid = Uuid::from_u128(20103);
const COMMANDER: Uuid = Uuid::from_u128(20102);
const OTHER_PILOT: Uuid = Uuid::from_u128(20104);
const OBSERVER: Uuid = Uuid::from_u128(11);
const RECIPIENT: Uuid = Uuid::from_u128(5);
const ENEMY: Uuid = Uuid::from_u128(51);

fn scenario() -> Scenario {
    sensor_relay_exercise_scenario()
}
fn local_tracks(game: &mut Game) -> Vec<Track> {
    game.simulation.projection_for(OBSERVER, Side::Blue).tracks
}
fn received_tracks(game: &mut Game) -> Vec<Track> {
    game.simulation.projection_for(RECIPIENT, Side::Blue).tracks
}
fn ticks(game: &mut Game, count: u64) {
    for _ in 0..count {
        advance_game_tick(game);
    }
}

#[test]
fn headquarters_learns_a_scoped_snapshot_only_after_report_delivery() {
    let mut game = game_from_scenario(scenario());
    advance_game_tick(&mut game);
    let local = local_tracks(&mut game);
    assert_eq!(local.len(), 1);
    assert_ne!(local[0].track_id, ENEMY);
    assert!(received_tracks(&mut game).is_empty());
    assert_eq!(game.network_messages.len(), 1);
    let record = &game.network_messages[0];
    assert_eq!(record.state, MessageState::InTransit);
    assert_eq!(record.message.profile_id, TRACK_REPORT_PROFILE);
    assert!(network_message_visible(record, &game.roles[&SOURCE_ROLE]));
    assert!(!network_message_visible(record, &game.roles[&COMMANDER]));
    assert!(!network_message_visible(record, &game.roles[&OTHER_PILOT]));
    assert!(!String::from_utf8(record.encoded_bytes.clone())
        .unwrap()
        .contains(&ENEMY.to_string()));
    assert!(record.encoded_bytes.len() + 24 <= 1_200);
    let original = local[0].clone();
    while game.simulation.tick() < 10 && received_tracks(&mut game).is_empty() {
        advance_game_tick(&mut game);
    }
    let received = received_tracks(&mut game);
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].observed_tick, 1);
    assert!(received[0].received_tick > received[0].observed_tick);
    assert_eq!(
        received[0].position.longitude_deg,
        original.position.longitude_deg
    );
    assert_ne!(received[0].track_id, original.track_id);
    assert_ne!(received[0].track_id, ENEMY);
    assert!(local_tracks(&mut game)[0].position.longitude_deg > original.position.longitude_deg);
    assert!(network_message_visible(
        &game.network_messages[0],
        &game.roles[&COMMANDER]
    ));
    assert!(game
        .simulation
        .projection_for(Uuid::from_u128(12), Side::Blue)
        .tracks
        .is_empty());
}

#[test]
fn severed_link_drops_reports_and_retries_a_new_observation_at_the_configured_interval() {
    let mut scenario = scenario();
    for channel in &mut scenario.network.channels {
        channel.state = CommunicationChannelState::Severed;
    }
    let mut game = game_from_scenario(scenario);
    ticks(&mut game, 10);
    assert_eq!(game.network_messages.len(), 1);
    assert_eq!(game.network_messages[0].state, MessageState::Dropped);
    assert_eq!(
        game.network_messages[0].message.fields["track"]["observed_tick"],
        1
    );
    assert_eq!(local_tracks(&mut game).len(), 1);
    assert!(received_tracks(&mut game).is_empty());
    assert!(!network_message_visible(
        &game.network_messages[0],
        &game.roles[&COMMANDER]
    ));
    advance_game_tick(&mut game);
    assert_eq!(game.network_messages.len(), 2);
    assert_eq!(
        game.network_messages[1].message.fields["track"]["observed_tick"],
        11
    );
    assert!(received_tracks(&mut game).is_empty());
}

#[test]
fn pending_slow_report_is_not_duplicated_and_an_expired_report_grants_no_knowledge() {
    let mut scenario = scenario();
    scenario
        .units
        .iter_mut()
        .find(|unit| unit.id == ENEMY)
        .unwrap()
        .flight_path = None;
    for channel in &mut scenario.network.channels {
        channel.bit_rate_bps = 1;
    }
    let mut game = game_from_scenario(scenario);
    ticks(&mut game, 25);
    assert_eq!(game.network_messages.len(), 1);
    assert_eq!(game.pending_deliveries.len(), 1);
    assert!(received_tracks(&mut game).is_empty());
    ticks(&mut game, 6);
    assert_eq!(game.network_messages.len(), 2);
    assert_eq!(game.network_messages[0].state, MessageState::Expired);
    assert_eq!(
        game.network_messages[1].message.fields["track"]["observed_tick"],
        31
    );
    assert!(received_tracks(&mut game).is_empty());
}

#[test]
fn paused_game_freezes_report_delivery_and_pacing_until_resume() {
    let mut game = game_from_scenario(scenario());
    advance_game_tick(&mut game);
    game.status = GameStatus::Paused;
    ticks(&mut game, 20);
    send_current_reports(&mut game);
    assert_eq!(game.simulation.tick(), 1);
    assert_eq!(game.network_messages.len(), 1);
    assert!(received_tracks(&mut game).is_empty());
    game.status = GameStatus::Running;
    ticks(&mut game, 5);
    assert_eq!(received_tracks(&mut game)[0].observed_tick, 1);
    assert_eq!(game.network_messages.len(), 1);
}

#[test]
fn event_store_failure_prevents_a_delivered_report_from_updating_knowledge() {
    let mut game = game_from_scenario(scenario());
    advance_game_tick(&mut game);
    game.network_event_path = Some(
        std::env::temp_dir()
            .join(format!(
                "absent-world-at-war-report-store-{}",
                Uuid::new_v4()
            ))
            .join("events.jsonl"),
    );
    ticks(&mut game, 10);
    assert_eq!(game.status, GameStatus::Paused);
    assert!(received_tracks(&mut game).is_empty());
    assert!(game.pending_deliveries.is_empty());
    assert_eq!(game.network_messages[0].state, MessageState::Dropped);
    game.status = GameStatus::Running;
    ticks(&mut game, 3);
    assert!(received_tracks(&mut game).is_empty());
}

#[test]
fn contact_loss_does_not_send_old_knowledge_as_new_measurements() {
    let mut game = game_from_scenario(scenario());
    ticks(&mut game, 35);
    let local = local_tracks(&mut game);
    assert_eq!(local.len(), 1);
    assert!(local[0].observed_tick < game.simulation.tick());
    assert_eq!(game.network_messages.len(), 3);
    assert_eq!(
        game.network_messages.last().unwrap().message.fields["track"]["observed_tick"],
        21
    );
    assert!(received_tracks(&mut game)[0].observed_tick <= 21);
}

#[test]
fn restored_receiver_gets_a_fresh_report_without_retroactive_hidden_observations() {
    let mut scenario = scenario();
    let post = scenario
        .units
        .iter_mut()
        .find(|unit| unit.id == RECIPIENT)
        .unwrap();
    let original = post.position;
    let clear = sim_core::GeoPose {
        longitude_deg: original.longitude_deg + 1.0,
        ..original
    };
    post.flight_path = Some(sim_core::CyclicFlightPath {
        period_ticks: 100,
        waypoints: vec![
            sim_core::FlightWaypoint {
                at_tick: 0,
                position: original,
            },
            sim_core::FlightWaypoint {
                at_tick: 10,
                position: clear,
            },
        ],
    });
    scenario.jamming_regions.push(sim_core::JammingRegion {
        id: "report-test-jammer".into(),
        name: "Fictional receiver jammer".into(),
        center: original,
        radius_m: 50_000.0,
        band: scenario
            .network
            .channels
            .iter()
            .find_map(|channel| channel.radio.as_ref().map(|radio| radio.band))
            .unwrap(),
        jammed: 1.0,
    });
    let mut game = game_from_scenario(scenario);
    ticks(&mut game, 5);
    assert!(received_tracks(&mut game).is_empty());
    assert_eq!(game.network_messages[0].state, MessageState::Dropped);
    ticks(&mut game, 10);
    let received = received_tracks(&mut game);
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].observed_tick, 11);
    assert!(received[0].received_tick > 11);
    assert_eq!(game.network_messages.len(), 2);
}

#[test]
fn missing_report_profile_pauses_the_game_with_an_operational_error() {
    let mut game = game_from_scenario(scenario());
    game.message_profiles.remove(TRACK_REPORT_PROFILE);
    advance_game_tick(&mut game);
    assert_eq!(game.status, GameStatus::Paused);
    assert!(game
        .operational_error
        .as_deref()
        .unwrap()
        .contains("track-report message profile"));
    assert!(received_tracks(&mut game).is_empty());
}
