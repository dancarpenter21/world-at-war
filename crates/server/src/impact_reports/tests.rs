use super::*;
use crate::authority_tests::game_from_scenario;
use sim_core::{CommunicationChannelState, OrderKind};
use sim_scenario::ImpactReportRoute;
const COMMANDER: Uuid = Uuid::from_u128(20102);
const PILOT: Uuid = Uuid::from_u128(20103);
const OTHER: Uuid = Uuid::from_u128(20104);
const PLAYER: Uuid = Uuid::from_u128(8001);
const SHOOTER: Uuid = Uuid::from_u128(11);
const POST: Uuid = Uuid::from_u128(5);

fn report_scenario() -> Scenario {
    let mut scenario = combat_training_scenario();
    scenario.impact_report_routes = vec![ImpactReportRoute {
        origin_role_id: PILOT,
        recipient_unit_id: POST,
        retry_interval_ticks: 3,
    }];
    scenario.reporting_window_ticks = 20;
    scenario
}
fn fixture(scenario: Scenario) -> Game {
    let mut game = game_from_scenario(scenario);
    for role in game.roles.values_mut() {
        role.owner = Some(PLAYER);
        role.lease_generation = 4;
    }
    game
}
fn tick(game: &mut Game) {
    advance_game_tick(game);
    intents::record_execution_results(game);
}
fn fire(game: &mut Game, role_id: Uuid, id: u128) -> PlayerIntent {
    let role = game.roles[&role_id].clone();
    let track = game
        .simulation
        .projection_for(role.location_unit_id, role.side)
        .tracks[0]
        .clone();
    let intent = PlayerIntent {
        intent_id: Uuid::from_u128(id),
        issuer_role: role_id,
        target: SHOOTER,
        kind: OrderKind::Engage {
            track_id: track.track_id,
        },
        requested_tick: game.simulation.tick() + 1,
    };
    intents::submit_player_intent(
        game,
        role_id,
        SubmitIntentRequest {
            player_id: PLAYER,
            lease_generation: 4,
            intent: intent.clone(),
        },
    )
    .unwrap();
    intent
}
fn until_complete(game: &mut Game) {
    for _ in 0..25 {
        if game.simulation.mission_complete() {
            return;
        }
        tick(game);
    }
    panic!("mission did not complete");
}
fn received(game: &mut Game) -> Vec<sim_core::combat::ReceivedImpactReport> {
    game.simulation
        .projection_for(POST, Side::Blue)
        .combat
        .unwrap()
        .received_impacts
}
fn debrief(game: &mut Game, role: Uuid) -> serde_json::Value {
    let role = game.roles[&role].clone();
    serde_json::to_value(intents::mission_debrief(game, &role)).unwrap()
}

#[test]
fn final_impact_report_arrives_after_the_combat_clock_stops_and_remains_scoped() {
    let mut game = fixture(report_scenario());
    for _ in 0..5 {
        tick(&mut game);
    }
    let intent = fire(&mut game, COMMANDER, 98001);
    until_complete(&mut game);
    let finished = game.simulation.tick();
    let pose = game
        .simulation
        .projection_for(SHOOTER, Side::Blue)
        .own_units[0]
        .position;
    assert_eq!(game.status, GameStatus::Running);
    assert!(received(&mut game).is_empty());
    let report = game
        .network_messages
        .iter()
        .find(|record| record.message.profile_id == IMPACT_REPORT_PROFILE)
        .unwrap();
    assert!(!network_message_visible(report, &game.roles[&COMMANDER]));
    let before = debrief(&mut game, COMMANDER);
    assert!(before["entries"][0]["hit"].is_null());
    assert!(before["entries"][0]["launch_tick"].is_number());
    assert!(debrief(&mut game, OTHER)["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    for _ in 0..20 {
        if game.status == GameStatus::Paused {
            break;
        }
        tick(&mut game);
        assert_eq!(game.simulation.tick(), finished);
    }
    assert_eq!(game.status, GameStatus::Paused);
    let reports = received(&mut game);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].report.hit);
    assert_eq!(reports[0].report.intent_id, intent.intent_id);
    assert!(reports[0].received_tick > finished);
    assert_eq!(
        game.simulation
            .projection_for(SHOOTER, Side::Blue)
            .own_units[0]
            .position
            .latitude_deg,
        pose.latitude_deg
    );
    assert!(game
        .simulation
        .projection_for(Uuid::from_u128(12), Side::Blue)
        .combat
        .unwrap()
        .received_impacts
        .is_empty());
    let after = debrief(&mut game, COMMANDER);
    assert_eq!(after["entries"][0]["hit"], true);
    assert!(
        after["entries"][0]["report_received_tick"]
            .as_u64()
            .unwrap()
            > finished
    );
    assert!(!after.to_string().contains(&Uuid::from_u128(51).to_string()));
    let count = game
        .network_messages
        .iter()
        .filter(|record| record.message.profile_id == IMPACT_REPORT_PROFILE)
        .count();
    assert_eq!(count, 1);
    game.simulation.receive_impact_report(
        POST,
        reports[0].report.clone(),
        reports[0].received_tick + 5,
    );
    assert_eq!(received(&mut game).len(), 1);
    assert_eq!(
        received(&mut game)[0].received_tick,
        reports[0].received_tick
    );
}

#[test]
fn severed_impact_reports_retry_a_bounded_number_without_disclosing_the_result() {
    let mut scenario = report_scenario();
    scenario
        .authority
        .policies
        .iter_mut()
        .find(|policy| policy.action == sim_core::ACTION_ENGAGE)
        .unwrap()
        .direct_role_ids
        .push(PILOT);
    let return_channels: BTreeSet<_> = scenario
        .communication_links
        .iter()
        .filter(|link| link.from_entity_id == SHOOTER && link.to_entity_id == POST)
        .map(|link| link.channel_id.clone())
        .collect();
    for channel in &mut scenario.network.channels {
        if return_channels.contains(&channel.id) {
            channel.state = CommunicationChannelState::Severed;
        }
    }
    let mut game = fixture(scenario);
    tick(&mut game);
    fire(&mut game, PILOT, 98002);
    until_complete(&mut game);
    let finished = game.simulation.tick();
    for _ in 0..25 {
        tick(&mut game);
    }
    assert_eq!(game.status, GameStatus::Paused);
    assert_eq!(game.simulation.tick(), finished);
    let reports: Vec<_> = game
        .network_messages
        .iter()
        .filter(|record| record.message.profile_id == IMPACT_REPORT_PROFILE)
        .collect();
    assert_eq!(reports.len(), usize::from(MAX_ATTEMPTS));
    assert!(reports
        .iter()
        .all(|record| record.state == MessageState::Dropped));
    assert!(received(&mut game).is_empty());
    assert!(debrief(&mut game, COMMANDER)["entries"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn expired_reports_and_failed_event_persistence_never_become_received_telemetry() {
    for persistence_failure in [false, true] {
        let mut game = fixture(report_scenario());
        for _ in 0..5 {
            tick(&mut game);
        }
        fire(&mut game, COMMANDER, 98003);
        while game.simulation.local_impact_reports(SHOOTER).is_empty()
            && !game.simulation.mission_complete()
        {
            if persistence_failure && game.simulation.tick() >= 9 {
                game.network_event_path = Some(std::env::temp_dir());
            }
            game.message_profiles
                .get_mut(IMPACT_REPORT_PROFILE)
                .unwrap()
                .expiry_ticks = 1;
            tick(&mut game);
            if game.status == GameStatus::Paused {
                break;
            }
        }
        for _ in 0..25 {
            tick(&mut game);
        }
        assert_eq!(game.status, GameStatus::Paused);
        assert!(received(&mut game).is_empty());
        if persistence_failure {
            assert!(game.operational_error.is_some());
        } else {
            assert!(game
                .network_messages
                .iter()
                .any(|record| record.message.profile_id == IMPACT_REPORT_PROFILE
                    && record.state == MessageState::Expired));
        }
    }
}

#[test]
fn timed_blackout_prevents_initial_reports_and_patrol_positions_require_reacquisition() {
    let mut game = fixture(contested_combat_scenario());
    tick(&mut game);
    let first = game.simulation.current_sensor_tracks(SHOOTER)[0].clone();
    assert!(game
        .simulation
        .projection_for(POST, Side::Blue)
        .own_units
        .iter()
        .any(|unit| unit.receiver_jammed));
    for _ in 0..5 {
        tick(&mut game);
    }
    assert!(game
        .simulation
        .projection_for(POST, Side::Blue)
        .tracks
        .is_empty());
    for _ in 0..9 {
        tick(&mut game);
    }
    let post = game.simulation.projection_for(POST, Side::Blue);
    assert!(!post.tracks.is_empty());
    assert!(post.jamming_regions.is_empty());
    assert!(post.tracks[0].position.latitude_deg > first.position.latitude_deg);
    fire(&mut game, COMMANDER, 98005);
    for _ in 0..15 {
        tick(&mut game);
    }
    assert!(!game.simulation.local_impact_reports(SHOOTER)[0].hit);
    assert!(received(&mut game).iter().any(|impact| !impact.report.hit));
    fire(&mut game, COMMANDER, 98004);
    until_complete(&mut game);
    for _ in 0..20 {
        tick(&mut game);
    }
    assert_eq!(game.status, GameStatus::Paused);
    assert_eq!(
        game.simulation
            .projection_for(POST, Side::Blue)
            .combat
            .unwrap()
            .mission
            .unwrap()
            .status,
        sim_core::combat::MissionStatus::Succeeded
    );
    assert_eq!(received(&mut game).len(), 2);
}

#[test]
fn invalid_report_subscriptions_and_jamming_windows_are_rejected() {
    let mut scenario = contested_combat_scenario();
    scenario.impact_report_routes[0].recipient_unit_id = Uuid::from_u128(51);
    assert!(scenario.validate().is_err());
    scenario = contested_combat_scenario();
    scenario.reporting_window_ticks = 0;
    assert!(scenario.validate().is_err());
    scenario = contested_combat_scenario();
    scenario.reporting_window_ticks = 61;
    assert!(scenario.validate().is_err());
    scenario = contested_combat_scenario();
    scenario.impact_report_routes[0].origin_role_id = OTHER;
    assert!(scenario.validate().is_err());
    scenario = contested_combat_scenario();
    scenario.impact_report_routes[0].retry_interval_ticks = 0;
    assert!(scenario.validate().is_err());
    scenario = contested_combat_scenario();
    scenario.jamming_regions[0].active_until_tick = Some(0);
    assert!(scenario.validate().is_err());
}

#[test]
fn reporting_window_ends_with_recorded_drops_when_packets_cannot_finish_in_time() {
    let mut scenario = report_scenario();
    scenario.reporting_window_ticks = 1;
    let mut game = fixture(scenario);
    for _ in 0..5 {
        tick(&mut game);
    }
    fire(&mut game, COMMANDER, 98006);
    until_complete(&mut game);
    let finished = game.simulation.tick();
    tick(&mut game);
    assert_eq!(game.status, GameStatus::Paused);
    assert_eq!(game.simulation.tick(), finished);
    assert!(game.pending_deliveries.is_empty());
    assert!(game
        .network_messages
        .iter()
        .any(|record| record.message.profile_id == IMPACT_REPORT_PROFILE
            && record.state == MessageState::Dropped
            && record.drop_reason.as_deref() == Some("mission reporting window ended")));
    assert!(received(&mut game).is_empty());
}

#[test]
fn a_delivered_report_adds_a_debrief_row_without_revealing_unreported_submission_history() {
    let mut scenario = report_scenario();
    scenario
        .authority
        .policies
        .iter_mut()
        .find(|policy| policy.action == sim_core::ACTION_ENGAGE)
        .unwrap()
        .direct_role_ids
        .push(PILOT);
    let mut game = fixture(scenario);
    tick(&mut game);
    fire(&mut game, PILOT, 98007);
    until_complete(&mut game);
    assert!(debrief(&mut game, COMMANDER)["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    for _ in 0..20 {
        tick(&mut game);
    }
    let after = debrief(&mut game, COMMANDER);
    assert_eq!(after["entries"].as_array().unwrap().len(), 1);
    assert_eq!(after["entries"][0]["hit"], true);
    assert!(after["entries"][0]["submitted_tick"].is_null());
    assert!(after["entries"][0]["approval_ticks"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(debrief(&mut game, OTHER)["entries"]
        .as_array()
        .unwrap()
        .is_empty());
    let count = game.network_messages.len();
    assert!(submit_authority_action(
        &mut game,
        COMMANDER,
        sim_core::ACTION_MOVE.into(),
        SHOOTER,
        String::new(),
        None
    )
    .is_err());
    assert_eq!(game.network_messages.len(), count);
}
