use sim_core::Side;
use sim_scenario::{command_link_exercise_scenario, regional_campaign_scenario};
use uuid::Uuid;

#[test]
fn observers_are_opt_in_and_cannot_collide_with_units_or_command_roles() {
    let mut scenario = regional_campaign_scenario();
    assert_eq!(scenario.observer_seats.len(), 2);
    scenario.validate().unwrap();
    assert!(command_link_exercise_scenario().observer_seats.is_empty());
    let original = scenario.observer_seats[0].id;
    for id in [
        scenario.units[0].id,
        scenario.authority.roles[0].id,
        scenario.observer_seats[1].id,
    ] {
        scenario.observer_seats[0].id = id;
        assert!(scenario.validate().is_err());
    }
    scenario.observer_seats[0].id = original;
    scenario.observer_seats[0].name.clear();
    assert!(scenario.validate().is_err());
}

#[test]
fn truth_reads_do_not_share_hidden_units_or_change_simulation_outcomes() {
    let mut scenario = regional_campaign_scenario();
    for unit in &mut scenario.units {
        unit.sensor = None;
    }
    let mut changed = scenario.clone();
    let enemy = changed
        .units
        .iter_mut()
        .find(|u| u.side == Side::Red)
        .unwrap();
    enemy.position.latitude_deg += 0.2;
    let mut inspected = scenario.spawn_with_seed(42).unwrap();
    let mut untouched = scenario.spawn_with_seed(42).unwrap();
    let mut different_truth = changed.spawn_with_seed(42).unwrap();
    for _ in 0..8 {
        let truth = inspected.truth_projection();
        assert!(truth.units.iter().any(|u| u.side == Side::Red));
        assert_eq!(
            serde_json::to_vec(&truth).unwrap(),
            serde_json::to_vec(&inspected.truth_projection()).unwrap()
        );
        assert_ne!(
            serde_json::to_vec(&truth).unwrap(),
            serde_json::to_vec(&different_truth.truth_projection()).unwrap()
        );
        for role in scenario
            .authority
            .roles
            .iter()
            .filter(|r| r.side == Side::Blue)
        {
            let observed =
                serde_json::to_vec(&inspected.projection_for(role.location_unit_id, role.side))
                    .unwrap();
            assert_eq!(
                observed,
                serde_json::to_vec(&untouched.projection_for(role.location_unit_id, role.side))
                    .unwrap()
            );
            assert_eq!(
                observed,
                serde_json::to_vec(
                    &different_truth.projection_for(role.location_unit_id, role.side)
                )
                .unwrap()
            );
        }
        inspected.step();
        untouched.step();
        different_truth.step();
    }
    // The reference run has received no truth reads before this final comparison.
    assert_eq!(
        serde_json::to_vec(&inspected.truth_projection()).unwrap(),
        serde_json::to_vec(&untouched.truth_projection()).unwrap()
    );
    assert!(!inspected
        .truth_projection()
        .units
        .iter()
        .any(|u| u.state.id == Uuid::from_u128(9001)));
}
