use super::*;
use sim_core::{propagation::GeometryStatus, DeliveryState, MessageOptions, PacketMetadata};

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn status(sim: &mut Simulation, from: u128, to: u128) -> sim_core::CommunicationLinkStatus {
    sim.truth_projection()
        .communication_links
        .into_iter()
        .find(|l| l.from_entity_id == id(from) && l.to_entity_id == id(to))
        .unwrap()
}
fn step(sim: &mut Simulation, ticks: u64) -> Vec<sim_core::DeliveryEvent> {
    let mut events = vec![];
    for _ in 0..ticks {
        sim.step();
        events.extend(sim.drain_deliveries());
    }
    events
}

#[test]
fn ridge_blocks_direct_radio_without_reporting_jamming_and_relay_restores_delivery() {
    let scenario = ridge_relay_exercise_scenario();
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    let direct = status(&mut sim, 5, 11);
    assert_eq!(direct.geometry_status, Some(GeometryStatus::TerrainBlocked));
    assert!(!direct.available);
    assert_eq!(direct.jammed, 0.0);
    assert!(!status(&mut sim, 5, 12).available);
    let message = id(90001);
    sim.send_message(message, id(5), id(11), vec![7; 1800], 50)
        .unwrap();
    assert!(step(&mut sim, 7)
        .iter()
        .all(|e| e.state != DeliveryState::Delivered));
    let events = step(&mut sim, 10);
    assert!(status(&mut sim, 5, 12).available);
    assert!(!status(&mut sim, 5, 11).available);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.id == message && e.state == DeliveryState::Delivered)
            .count(),
        1
    );
    assert!(events
        .iter()
        .any(|e| e.id == message && e.state == DeliveryState::Acknowledged));
    sim.send_message(message, id(5), id(11), vec![7; 1800], 50)
        .unwrap();
    assert!(!step(&mut sim, 10)
        .iter()
        .any(|e| e.state == DeliveryState::Delivered));
}

#[test]
fn radio_antenna_height_movement_and_unknown_terrain_are_deterministic() {
    let mut scenario = ridge_relay_exercise_scenario();
    for link in &mut scenario.communication_links {
        let c = link.propagation.as_mut().unwrap();
        c.source_antenna_height_m = 4000.0;
        c.destination_antenna_height_m = 4000.0;
    }
    assert!(status(&mut scenario.spawn_with_seed(42).unwrap(), 5, 11).available);
    scenario
        .sensing
        .as_mut()
        .unwrap()
        .terrain_fixture
        .as_mut()
        .unwrap()
        .heights
        .fill(i16::MIN);
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    assert_eq!(
        status(&mut sim, 5, 11).geometry_status,
        Some(GeometryStatus::TerrainUnknown)
    );
    assert!(!status(&mut sim, 5, 11).available);
    scenario.sensing.as_mut().unwrap().terrain_fixture = None;
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    assert_eq!(
        status(&mut sim, 5, 11).geometry_status,
        Some(GeometryStatus::TerrainUnknown)
    );
    for l in &mut scenario.communication_links {
        l.propagation.as_mut().unwrap().terrain = false;
    }
    assert!(status(&mut scenario.spawn_with_seed(42).unwrap(), 5, 11).available);
    // A moved endpoint outside installed coverage is unknown, never assumed flat.
    let mut scenario = ridge_relay_exercise_scenario();
    scenario
        .units
        .iter_mut()
        .find(|u| u.id == id(11))
        .unwrap()
        .position
        .longitude_deg = -0.01;
    assert_eq!(
        status(&mut scenario.spawn_with_seed(42).unwrap(), 5, 11).geometry_status,
        Some(GeometryStatus::TerrainUnknown)
    );
}

#[test]
fn earth_horizon_range_and_authored_channel_state_remain_independent() {
    let mut scenario = ridge_relay_exercise_scenario();
    for link in &mut scenario.communication_links {
        let c = link.propagation.as_mut().unwrap();
        c.terrain = false;
        c.max_range_m = 5_000_000.0;
    }
    scenario
        .units
        .iter_mut()
        .find(|u| u.id == id(11))
        .unwrap()
        .position
        .longitude_deg = 10.0;
    assert_eq!(
        status(&mut scenario.spawn_with_seed(42).unwrap(), 5, 11).geometry_status,
        Some(GeometryStatus::EarthBlocked)
    );
    for l in &mut scenario.communication_links {
        l.propagation.as_mut().unwrap().max_range_m = 1.0;
    }
    assert_eq!(
        status(&mut scenario.spawn_with_seed(42).unwrap(), 5, 11).geometry_status,
        Some(GeometryStatus::OutOfRange)
    );
    let mut scenario = ridge_relay_exercise_scenario();
    let channel = scenario
        .communication_links
        .iter()
        .find(|l| l.from_entity_id == id(5) && l.to_entity_id == id(12))
        .unwrap()
        .channel_id
        .clone();
    scenario
        .network
        .channels
        .iter_mut()
        .find(|c| c.id == channel)
        .unwrap()
        .state = ChannelState::Degraded {
        effective_bit_rate_bps: 500_000,
    };
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    step(&mut sim, 8);
    assert_eq!(
        status(&mut sim, 5, 12).effective_bit_rate_bps,
        Some(500_000)
    );
    let mut scenario = ridge_relay_exercise_scenario();
    scenario
        .network
        .channels
        .iter_mut()
        .find(|c| c.id == channel)
        .unwrap()
        .state = ChannelState::Severed;
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    step(&mut sim, 8);
    assert!(!status(&mut sim, 5, 12).available);
}

#[test]
fn broken_return_path_cannot_acknowledge_a_reassembled_command() {
    let mut scenario = ridge_relay_exercise_scenario();
    for l in &mut scenario.communication_links {
        if l.to_entity_id == id(5) {
            l.propagation.as_mut().unwrap().max_range_m = 1.0;
        }
    }
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    step(&mut sim, 8);
    let message = id(90002);
    sim.send_message(message, id(5), id(11), vec![8; 1400], 50)
        .unwrap();
    let events = step(&mut sim, 20);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.state == DeliveryState::Delivered)
            .count(),
        1
    );
    assert!(!events
        .iter()
        .any(|e| e.state == DeliveryState::Acknowledged));
    assert!(events
        .iter()
        .any(|e| e.state == DeliveryState::Unacknowledged));
}

#[test]
fn hop_budget_expiry_and_exhausted_retries_prevent_late_delivery() {
    for (expires, hops) in [(5, 8), (50, 1)] {
        let mut sim = ridge_relay_exercise_scenario().spawn_with_seed(42).unwrap();
        sim.send_message_with_options(
            id(90003),
            id(5),
            id(11),
            vec![8; 1400],
            expires,
            Some(MessageOptions {
                hop_limit: hops,
                metadata: PacketMetadata::default(),
            }),
        )
        .unwrap();
        assert!(!step(&mut sim, 25)
            .iter()
            .any(|e| e.state == DeliveryState::Delivered));
    }
    let mut scenario = ridge_relay_exercise_scenario();
    scenario
        .units
        .iter_mut()
        .find(|u| u.id == id(12))
        .unwrap()
        .flight_path = None;
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    sim.send_message(id(90004), id(5), id(11), vec![8; 1400], 50)
        .unwrap();
    assert!(step(&mut sim, 25)
        .iter()
        .any(|e| e.state == DeliveryState::Dropped));
}

#[test]
fn snapshot_preserves_pending_radio_and_legacy_definitions_omit_new_fields() {
    let mut sim = ridge_relay_exercise_scenario().spawn_with_seed(42).unwrap();
    sim.send_message(id(90005), id(5), id(11), vec![9; 1600], 50)
        .unwrap();
    step(&mut sim, 4);
    let bytes = serde_json::to_vec(&sim.snapshot()).unwrap();
    let mut restored = Simulation::restore(serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(bytes, serde_json::to_vec(&restored.snapshot()).unwrap());
    assert_eq!(
        serde_json::to_value(step(&mut sim, 20)).unwrap(),
        serde_json::to_value(step(&mut restored, 20)).unwrap()
    );
    let legacy = sensor_relay_exercise_scenario();
    assert!(!serde_json::to_string(&legacy)
        .unwrap()
        .contains("\"propagation\""));
    let mut legacy = legacy.spawn_with_seed(42).unwrap();
    assert!(!legacy.uses_radio_propagation());
    assert!(!serde_json::to_string(&legacy.snapshot())
        .unwrap()
        .contains("\"propagation\""));
    assert!(status(&mut legacy, 5, 11).geometry_status.is_none());
}

#[test]
fn campaign_upgrade_is_explicit_and_validates_eligible_radio_links() {
    for scenario in [
        regional_campaign_scenario(),
        iftu_exercise_scenario(),
        global_crisis_scenario(),
    ] {
        let prior = upgrade_sensing(scenario);
        let upgraded = upgrade_radio_propagation(prior.clone());
        upgraded.validate().unwrap();
        assert_eq!(upgraded.version, prior.version + 1);
        for l in &upgraded.communication_links {
            let channel = upgraded
                .network
                .channels
                .iter()
                .find(|c| c.id == l.channel_id)
                .unwrap();
            if let Some(c) = &l.propagation {
                assert!(channel.radio.is_some());
                assert_eq!(c.terrain, prior.campaign.is_some());
            }
        }
        assert!(prior
            .communication_links
            .iter()
            .all(|l| l.propagation.is_none()));
    }
    let mut scenario = ridge_relay_exercise_scenario();
    scenario.communication_links[0]
        .propagation
        .as_mut()
        .unwrap()
        .source_antenna_height_m = -1.0;
    assert!(scenario.validate().is_err());
}

#[test]
fn fractional_restore_retains_radio_boundary_geometry_until_the_next_second() {
    let mut sim = ridge_relay_exercise_scenario().spawn_with_seed(42).unwrap();
    for _ in 0..450 {
        sim.step_frame(1.0).unwrap();
    }
    assert_eq!(sim.elapsed_game_seconds(), 7.5);
    sim.send_message(id(90006), id(5), id(11), vec![4; 1600], 50)
        .unwrap();
    let before = sim.truth_projection();
    let bytes = serde_json::to_vec(&sim.snapshot()).unwrap();
    let mut restored = Simulation::restore(serde_json::from_slice(&bytes).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.truth_projection()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(bytes, serde_json::to_vec(&restored.snapshot()).unwrap());
    for _ in 0..1000 {
        sim.step_frame(1.0).unwrap();
        restored.step_frame(1.0).unwrap();
    }
    assert_eq!(
        serde_json::to_vec(&sim.snapshot()).unwrap(),
        serde_json::to_vec(&restored.snapshot()).unwrap()
    );
}

#[test]
fn campaign_projection_does_not_expose_geometry_for_unreported_remote_terminals() {
    let mut sim = ridge_relay_exercise_scenario().spawn_with_seed(42).unwrap();
    sim.enable_campaign();
    let projection = sim.projection_for(id(5), Side::Blue);
    assert_eq!(projection.own_units.len(), 1);
    assert!(projection.communication_links.is_empty());
    assert_eq!(sim.truth_projection().communication_links.len(), 6);
}

#[test]
fn nearby_relay_without_a_configured_forward_link_cannot_restore_delivery() {
    let mut scenario = ridge_relay_exercise_scenario();
    scenario
        .communication_links
        .retain(|l| !(l.from_entity_id == id(12) && l.to_entity_id == id(11)));
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    sim.send_message(id(90007), id(5), id(11), vec![1; 1000], 50)
        .unwrap();
    let events = step(&mut sim, 25);
    assert!(status(&mut sim, 5, 12).available);
    assert!(!events.iter().any(|e| e.state == DeliveryState::Delivered));
    assert!(events.iter().any(|e| e.state == DeliveryState::Dropped));
}
