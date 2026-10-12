use sim_core::{sensing::Modality, Side};
use sim_scenario::sensing_exercise_scenario;
use uuid::Uuid;
#[test]
fn synthetic_terrain_exercise_is_offline_and_recovery_keeps_noisy_knowledge() {
    let scenario = sensing_exercise_scenario();
    scenario.validate().unwrap();
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    sim.step();
    let owner = Uuid::from_u128(12);
    let a = sim.projection_for(owner, Side::Blue);
    assert!(a.sensing.is_some());
    let mut restored = sim_core::Simulation::restore(sim.snapshot()).unwrap();
    for _ in 0..10 {
        sim.step();
        restored.step();
        assert_eq!(
            serde_json::to_vec(&sim.projection_for(owner, Side::Blue)).unwrap(),
            serde_json::to_vec(&restored.projection_for(owner, Side::Blue)).unwrap()
        );
    }
}
#[test]
fn raising_observer_over_ridge_reacquires_contact_without_sharing_truth() {
    let mut scenario = sensing_exercise_scenario();
    // Make this a single-modality geometric fixture with certain in-range detection.
    let config = scenario.sensing.as_mut().unwrap();
    for sensor in config.sensors.values_mut() {
        sensor.modalities = vec![Modality::Radar];
    }
    let owner = Uuid::from_u128(11);
    let post = Uuid::from_u128(5);
    let mut masked = scenario.spawn_with_seed(42).unwrap();
    masked.step();
    assert!(masked.projection_for(owner, Side::Blue).tracks.is_empty());
    assert!(masked.projection_for(post, Side::Blue).tracks.is_empty());
    scenario
        .units
        .iter_mut()
        .find(|u| u.id == owner)
        .unwrap()
        .position
        .altitude_m = 5000.0;
    let mut clear = scenario.spawn_with_seed(42).unwrap();
    clear.step();
    assert!(!clear.projection_for(owner, Side::Blue).tracks.is_empty());
    assert!(clear.projection_for(post, Side::Blue).tracks.is_empty());
}
#[test]
fn nearby_emitting_contact_produces_a_passive_bearing() {
    let scenario = sensing_exercise_scenario();
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    sim.step();
    assert!(!sim
        .projection_for(Uuid::from_u128(12), Side::Blue)
        .sensing
        .unwrap()
        .bearings
        .is_empty());
}

#[test]
fn slow_control_message_bounds_internal_report_backlog_on_its_route() {
    let scenario = sensing_exercise_scenario();
    let link = scenario.communication_links.first().unwrap().clone();
    let mut sim = scenario.spawn_with_seed(42).unwrap();
    sim.enable_campaign();
    let id = Uuid::from_u128(987654);
    sim.send_message(
        id,
        link.from_entity_id,
        link.to_entity_id,
        vec![0; 128 * 1024],
        200,
    )
    .unwrap();
    for _ in 0..12 {
        sim.step();
    }
    let snapshot = serde_json::to_value(sim.snapshot()).unwrap();
    let messages = snapshot["transport"]["messages"].as_object().unwrap();
    let competing = messages
        .values()
        .filter(|message| {
            message["from"] == link.from_entity_id.to_string()
                && message["to"] == link.to_entity_id.to_string()
        })
        .count();
    assert_eq!(
        competing, 1,
        "the slow control message must not accumulate full overlapping knowledge reports"
    );
    assert!(messages.contains_key(&id.to_string()));
}
