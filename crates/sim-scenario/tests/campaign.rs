use sim_core::{operations::*, DeliveryState, GeoPose, Side};
use sim_scenario::regional_campaign_scenario;
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

#[test]
fn campaign_has_two_legal_alternatives_and_separate_controller_authority() {
    let scenario = regional_campaign_scenario();
    scenario.validate().unwrap();
    let plan = scenario.campaign.unwrap();
    assert_eq!(plan.comparisons().len(), 2);
    assert!(!scenario.authority.role_is_in_unit_chain(id(205), id(11)));
    assert!(scenario.authority.role_is_in_unit_chain(id(201), id(11)));
    assert_eq!(plan.airspaces.len(), 2);
}

#[test]
fn rejects_overlapping_controller_assignments_and_duplicate_aircraft_allocations() {
    let scenario = regional_campaign_scenario();
    let units = scenario.units.iter().map(|u| u.id).collect();
    let mut plan = scenario.campaign.clone().unwrap();
    plan.airspaces[1].polygon = plan.airspaces[0].polygon.clone();
    assert!(plan
        .validate(&scenario.authority, &units)
        .unwrap_err()
        .contains("overlapping sector"));
    let mut plan = scenario.campaign.unwrap();
    plan.courses[0].missions[1].unit_id = plan.courses[0].missions[0].unit_id;
    assert!(plan
        .validate(&scenario.authority, &units)
        .unwrap_err()
        .contains("overlapping missions"));
}

#[test]
fn message_submission_never_advances_time_and_fragments_execute_once() {
    let scenario = regional_campaign_scenario();
    let mut simulation = scenario.spawn().unwrap();
    let payload = vec![42; 4096];
    simulation
        .send_message(id(900), id(1), id(11), payload.clone(), 100)
        .unwrap();
    assert_eq!(simulation.tick(), 0);
    assert!(simulation.drain_deliveries().is_empty());
    simulation
        .send_message(id(900), id(1), id(11), payload.clone(), 100)
        .unwrap();
    let mut delivered = 0;
    let mut acked = 0;
    for _ in 0..20 {
        simulation.step();
        for event in simulation
            .drain_deliveries()
            .into_iter()
            .filter(|e| e.id == id(900))
        {
            if event.state == DeliveryState::Delivered {
                assert_eq!(event.payload, payload);
                delivered += 1;
            }
            if event.state == DeliveryState::Acknowledged {
                acked += 1;
            }
        }
    }
    assert_eq!(delivered, 1);
    assert_eq!(acked, 1);
}

#[test]
fn no_remote_friendly_positions_until_reports_arrive_and_tracks_hide_truth_ids() {
    let mut simulation = regional_campaign_scenario().spawn().unwrap();
    assert_eq!(
        simulation.projection_for(id(1), Side::Blue).own_units.len(),
        1
    );
    for _ in 0..10 {
        simulation.step();
    }
    let picture = simulation.projection_for(id(1), Side::Blue);
    assert!(picture.own_units.len() > 1);
    assert!(picture.jamming_regions.is_empty());
    assert!(picture
        .tracks
        .iter()
        .all(|t| ![id(51), id(61), id(62)].contains(&t.track_id)));
    assert!(picture.own_units.iter().all(|u| u.id != id(51)));
}

#[test]
fn earth_horizon_blocks_low_observers_but_allows_airborne_observers() {
    let low = GeoPose {
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        altitude_m: 0.0,
    };
    let target = GeoPose {
        longitude_deg: 1.0,
        ..low
    };
    assert!(!sim_core::line_of_sight(low, target));
    assert!(sim_core::line_of_sight(
        GeoPose {
            altitude_m: 8000.0,
            ..low
        },
        target
    ));
}

#[test]
fn lost_acknowledgement_does_not_redeliver_the_order() {
    let mut scenario = regional_campaign_scenario();
    scenario
        .communication_links
        .retain(|link| link.to_entity_id != id(1) || link.from_entity_id == id(11));
    scenario.jamming_regions.push(sim_core::JammingRegion {
        id: "ack-outage".into(),
        name: "Acknowledgement outage".into(),
        center: scenario
            .units
            .iter()
            .find(|u| u.id == id(1))
            .unwrap()
            .position,
        radius_m: 1000.0,
        band: c3mesh::FrequencyBand::new(960_000_000, 1_215_000_000),
        jammed: 1.0,
    });
    let mut simulation = scenario.spawn().unwrap();
    simulation
        .send_message(id(901), id(1), id(11), vec![7; 1024], 100)
        .unwrap();
    let mut states = Vec::new();
    for _ in 0..25 {
        simulation.step();
        states.extend(
            simulation
                .drain_deliveries()
                .into_iter()
                .filter(|e| e.id == id(901))
                .map(|e| e.state),
        );
    }
    assert_eq!(
        states
            .iter()
            .filter(|s| **s == DeliveryState::Delivered)
            .count(),
        1
    );
    assert!(states.contains(&DeliveryState::Unacknowledged));
    assert!(!states.contains(&DeliveryState::Acknowledged));
}

#[test]
fn expired_order_is_never_delivered() {
    let mut simulation = regional_campaign_scenario().spawn().unwrap();
    simulation
        .send_message(id(902), id(1), id(11), vec![1], 1)
        .unwrap();
    simulation.step();
    let states: Vec<_> = simulation
        .drain_deliveries()
        .into_iter()
        .filter(|e| e.id == id(902))
        .map(|e| e.state)
        .collect();
    assert_eq!(states, vec![DeliveryState::Expired]);
}

#[test]
fn mission_execution_requires_receipt_and_cancellation_is_local() {
    let scenario = regional_campaign_scenario();
    let mut plan = scenario.campaign.clone().unwrap();
    plan.selected_course_id = Some(plan.courses[0].id);
    let (aco, ato) = plan.products().unwrap();
    let mut simulation = scenario.spawn().unwrap();
    assert!(simulation.mission_reports(id(11)).is_empty());
    simulation.receive_tasking(id(11), &aco, &ato);
    simulation.receive_tasking(id(12), &aco, &ato);
    assert_eq!(
        simulation.mission_reports(id(11))[0].state,
        MissionState::Scheduled
    );
    simulation.cancel_missions(id(11), &[ato.missions[0].id]);
    assert_eq!(
        simulation.mission_reports(id(11))[0].state,
        MissionState::Cancelled
    );
    assert_eq!(
        simulation.mission_reports(id(12))[0].state,
        MissionState::Scheduled
    );
}

#[test]
fn equal_seed_and_inputs_reproduce_mission_outcomes() {
    let scenario = regional_campaign_scenario();
    let mut plan = scenario.campaign.clone().unwrap();
    plan.selected_course_id = Some(plan.courses[0].id);
    let (aco, ato) = plan.products().unwrap();
    let run = || {
        let mut sim = scenario.spawn_with_seed(42).unwrap();
        sim.receive_tasking(id(11), &aco, &ato);
        for _ in 0..220 {
            sim.step();
            sim.drain_deliveries();
        }
        (
            serde_json::to_value(sim.mission_reports(id(11))).unwrap(),
            serde_json::to_value(sim.projection_for(id(11), Side::Blue)).unwrap(),
        )
    };
    assert_eq!(run(), run());
}

#[test]
fn exhausted_aircraft_stops_instead_of_coasting_after_mission_failure() {
    let scenario = regional_campaign_scenario();
    let mut plan = scenario.campaign.clone().unwrap();
    plan.selected_course_id = Some(plan.courses[0].id);
    let (aco, mut ato) = plan.products().unwrap();
    ato.missions.retain(|m| m.unit_id == id(11));
    ato.missions[0].start_tick = 1;
    ato.missions[0].depends_on.clear();
    let mut sim = scenario.spawn().unwrap();
    sim.set_combat_profile(
        id(11),
        CombatProfile {
            fuel_seconds: 1.0,
            ..Default::default()
        },
    )
    .unwrap();
    sim.receive_tasking(id(11), &aco, &ato);
    sim.step();
    let position = sim
        .projection_for(id(11), Side::Blue)
        .own_units
        .into_iter()
        .find(|u| u.id == id(11))
        .unwrap()
        .position;
    sim.step();
    assert_eq!(sim.mission_reports(id(11))[0].state, MissionState::Failed);
    let stopped = sim
        .projection_for(id(11), Side::Blue)
        .own_units
        .into_iter()
        .find(|u| u.id == id(11))
        .unwrap()
        .position;
    assert_eq!(
        serde_json::to_value(position).unwrap(),
        serde_json::to_value(stopped).unwrap()
    );
}
