use super::*;

const SHOOTER: Uuid = Uuid::from_u128(1);
const ENEMY: Uuid = Uuid::from_u128(2);
const COMMANDER: Uuid = Uuid::from_u128(3);

fn fixture(ammunition: u32, damage: u32, deadline_tick: u64) -> Simulation {
    let platforms: Vec<_> = [SHOOTER, ENEMY, COMMANDER]
        .into_iter()
        .map(|id| PlatformSpawn {
            id,
            name: "Combat fixture".into(),
            side: if id == ENEMY { Side::Red } else { Side::Blue },
            domain: Domain::Air,
            pose: GeoPose {
                latitude_deg: 0.0,
                longitude_deg: if id == ENEMY { 0.01 } else { 0.0 },
                altitude_m: 1_000.0,
            },
            velocity: Velocity::default(),
            sensor: (id == SHOOTER).then_some(Sensor {
                scan_interval_ticks: 1,
                field_of_regard_deg: 360.0,
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
            network_device_ids: vec![DeviceId::new(id.to_string())],
            flight_path: None,
            sidc: unknown_sidc(Side::Blue).into(),
        })
        .collect();
    let communications = CommunicationsConfig {
        network: NetworkConfig {
            devices: platforms
                .iter()
                .flat_map(|unit| &unit.network_device_ids)
                .map(|id| c3mesh::DeviceConfig {
                    id: id.clone(),
                    kind: DeviceKind::Sink,
                    mobility: Default::default(),
                    interference: vec![],
                })
                .collect(),
            channels: vec![],
        },
        simulator_options: SimulatorOptions::default(),
        links: vec![],
        jamming_regions: vec![],
    };
    let mut sim = Simulation::new(platforms, communications).unwrap();
    sim.configure_combat(config(ammunition, damage, deadline_tick))
        .unwrap();
    sim
}

fn config(ammunition: u32, damage: u32, deadline_tick: u64) -> CombatConfig {
    CombatConfig {
        units: vec![
            CombatUnitDefinition {
                unit_id: SHOOTER,
                hit_points: 100,
                weapon: Some(TrainingWeapon {
                    ammunition,
                    damage,
                    range_m: 10_000.0,
                    speed_mps: 500.0,
                    blast_radius_m: 30.0,
                    max_track_age_ticks: 5,
                }),
            },
            CombatUnitDefinition {
                unit_id: ENEMY,
                hit_points: 100,
                weapon: None,
            },
        ],
        mission: Some(TrainingMission {
            title: "Destroy the training target".into(),
            side: Side::Blue,
            target_unit_ids: vec![ENEMY],
            deadline_tick,
        }),
    }
}

fn fire(sim: &mut Simulation, id: u128, requested_tick: u64) {
    let track = sim.current_sensor_tracks(SHOOTER)[0].clone();
    let intent_id = Uuid::from_u128(id);
    sim.designate_engagement(intent_id, SHOOTER, SHOOTER, track.track_id)
        .unwrap();
    sim.queue_authorized_intent(AuthorizedIntent {
        intent: PlayerIntent {
            intent_id,
            issuer_role: Uuid::from_u128(10),
            target: SHOOTER,
            kind: OrderKind::Engage {
                track_id: track.track_id,
            },
            requested_tick,
        },
        authorization: AuthorizationRecord {
            policy_id: Uuid::from_u128(11),
            policy_version: 1,
            requester_role_id: Uuid::from_u128(10),
            granting_role_id: Uuid::from_u128(10),
            request_id: None,
        },
    });
}

fn ammunition(sim: &mut Simulation) -> u32 {
    sim.projection_for(SHOOTER, Side::Blue)
        .own_units
        .iter()
        .find(|unit| unit.id == SHOOTER)
        .unwrap()
        .weapon
        .as_ref()
        .unwrap()
        .ammunition
}

#[test]
fn weapon_consumes_one_round_at_launch_and_destroys_only_at_the_impact_tick() {
    let mut sim = fixture(2, 100, 60);
    sim.step();
    fire(&mut sim, 100, 0);
    assert_eq!(ammunition(&mut sim), 2);
    sim.step();
    assert_eq!(ammunition(&mut sim), 1);
    assert!(matches!(
        sim.drain_order_results()[0].status,
        OrderStatus::Accepted
    ));
    for _ in 0..2 {
        sim.step();
    }
    assert_eq!(
        sim.world.resource::<CombatState>().hit_points(ENEMY),
        Some(100)
    );
    sim.step();
    assert_eq!(sim.tick(), 5);
    assert_eq!(
        sim.world.resource::<CombatState>().hit_points(ENEMY),
        Some(0)
    );
    assert!(sim.projection_for(ENEMY, Side::Red).own_units.is_empty());
    let projection = sim.projection_for(SHOOTER, Side::Blue);
    let combat = projection.combat.unwrap();
    assert_eq!(combat.mission.unwrap().status, MissionStatus::Succeeded);
    assert_eq!(combat.local_impacts.len(), 1);
    assert!(combat.local_impacts[0].hit);
    assert_eq!(combat.local_impacts[0].resolved_tick, 5);
    assert!(
        !serde_json::to_string(&sim.projection_for(SHOOTER, Side::Blue))
            .unwrap()
            .contains(&ENEMY.to_string())
    );
    let commander = sim.projection_for(COMMANDER, Side::Blue).combat.unwrap();
    assert!(commander.local_impacts.is_empty());
    assert!(sim
        .projection_for(ENEMY, Side::Red)
        .combat
        .unwrap()
        .mission
        .is_none());
}

#[test]
fn damage_accumulates_without_destroying_a_target_after_a_partial_hit() {
    let mut sim = fixture(2, 60, 60);
    sim.step();
    fire(&mut sim, 101, 0);
    for _ in 0..4 {
        sim.step();
    }
    assert_eq!(
        sim.world.resource::<CombatState>().hit_points(ENEMY),
        Some(40)
    );
    assert!(!sim.mission_complete());
    fire(&mut sim, 102, 0);
    for _ in 0..4 {
        sim.step();
    }
    assert_eq!(
        sim.world.resource::<CombatState>().hit_points(ENEMY),
        Some(0)
    );
    assert_eq!(ammunition(&mut sim), 0);
    assert_eq!(
        sim.projection_for(SHOOTER, Side::Blue)
            .combat
            .unwrap()
            .mission
            .unwrap()
            .status,
        MissionStatus::Succeeded
    );
}

#[test]
fn a_moving_target_escapes_the_frozen_aim_point_and_exhaustion_fails_the_mission() {
    let mut sim = fixture(1, 100, 60);
    sim.step();
    fire(&mut sim, 103, 0);
    sim.step();
    let mut query = sim
        .world
        .query::<(&SimEntityId, &mut GeoPose, &mut GeodesicMotionState)>();
    let (_, mut pose, mut motion) = query
        .iter_mut(&mut sim.world)
        .find(|(id, _, _)| id.0 == ENEMY)
        .unwrap();
    pose.longitude_deg = 0.03;
    motion.origin = *pose;
    for _ in 0..3 {
        sim.step();
    }
    let combat = sim.projection_for(SHOOTER, Side::Blue).combat.unwrap();
    assert!(!combat.local_impacts[0].hit);
    assert_eq!(combat.mission.unwrap().status, MissionStatus::Failed);
    assert_eq!(
        sim.world.resource::<CombatState>().hit_points(ENEMY),
        Some(100)
    );
}

#[test]
fn delayed_orders_reject_the_original_stale_report_even_with_a_fresh_live_contact() {
    let mut sim = fixture(2, 100, 60);
    sim.step();
    fire(&mut sim, 104, 7);
    for _ in 0..6 {
        sim.step();
    }
    let result = sim.drain_order_results().pop().unwrap();
    assert!(
        matches!(result.status, OrderStatus::Rejected(ref reason) if reason.contains("too old"))
    );
    assert_eq!(ammunition(&mut sim), 2);
    assert_eq!(sim.current_sensor_tracks(SHOOTER)[0].observed_tick, 7);
}

#[test]
fn unknown_or_another_terminals_track_cannot_designate_a_target_until_a_report_arrives() {
    let mut sim = fixture(2, 100, 60);
    sim.step();
    let local = sim.current_sensor_tracks(SHOOTER)[0].clone();
    assert!(sim
        .designate_engagement(Uuid::from_u128(105), SHOOTER, SHOOTER, ENEMY)
        .is_err());
    assert!(sim
        .designate_engagement(Uuid::from_u128(105), SHOOTER, COMMANDER, local.track_id)
        .is_err());
    assert!(sim.receive_track_report(COMMANDER, local.clone()));
    let received = sim.projection_for(COMMANDER, Side::Blue).tracks[0].clone();
    sim.designate_engagement(Uuid::from_u128(105), SHOOTER, COMMANDER, received.track_id)
        .unwrap();
    assert_eq!(
        sim.engagement_designation(Uuid::from_u128(105))
            .unwrap()
            .position
            .longitude_deg,
        local.position.longitude_deg
    );
}

#[test]
fn out_of_range_launch_rejects_without_consuming_ammunition() {
    let mut sim = fixture(2, 100, 60);
    sim.world
        .resource_mut::<CombatState>()
        .weapons
        .get_mut(&SHOOTER)
        .unwrap()
        .range_m = 500.0;
    sim.step();
    fire(&mut sim, 106, 0);
    sim.step();
    assert!(
        matches!(sim.drain_order_results()[0].status, OrderStatus::Rejected(ref reason) if reason.contains("range"))
    );
    assert_eq!(ammunition(&mut sim), 2);
}

#[test]
fn the_time_limit_has_a_stable_failure_outcome_and_rejects_further_launches() {
    let mut sim = fixture(2, 100, 2);
    sim.step();
    sim.step();
    let mission = sim
        .projection_for(SHOOTER, Side::Blue)
        .combat
        .unwrap()
        .mission
        .unwrap();
    assert_eq!(mission.status, MissionStatus::Failed);
    assert_eq!(mission.finished_tick, Some(2));
    fire(&mut sim, 107, 0);
    sim.step();
    assert!(
        matches!(sim.drain_order_results()[0].status, OrderStatus::Rejected(ref reason) if reason.contains("ended"))
    );
    assert_eq!(ammunition(&mut sim), 2);
    assert_eq!(
        sim.projection_for(SHOOTER, Side::Blue)
            .combat
            .unwrap()
            .mission
            .unwrap()
            .finished_tick,
        Some(2)
    );
}

#[test]
fn reporting_after_mission_completion_compacts_without_advancing_combat() {
    let mut compact = fixture(2, 100, 2);
    let mut full = fixture(2, 100, 2);
    full.compact_network_history = false;
    for sim in [&mut compact, &mut full] {
        sim.step();
        sim.step();
        assert!(sim.mission_complete());
    }
    for _ in 0..100 {
        for sim in [&mut compact, &mut full] {
            sim.advance_reporting_clock();
            assert!(sim.advance_network().unwrap().is_empty());
            assert_eq!(sim.tick(), 2);
        }
        assert_eq!(
            serde_json::to_value(compact.projection_for(SHOOTER, Side::Blue)).unwrap(),
            serde_json::to_value(full.projection_for(SHOOTER, Side::Blue)).unwrap()
        );
    }
    let counts = compact.retention_statistics().network;
    assert_eq!(counts.retained_from_ns, 102_000_000_000);
    assert_eq!(counts.interference_entries, counts.devices);
    assert!(
        full.retention_statistics().network.interference_entries > counts.interference_entries * 50
    );
}

#[test]
fn invalid_weapon_and_mission_definitions_are_rejected_before_spawn() {
    let sides = BTreeMap::from([(SHOOTER, Side::Blue), (ENEMY, Side::Red)]);
    for kind in 0..6 {
        let mut invalid = config(2, 100, 60);
        match kind {
            0 => invalid.units[0].weapon.as_mut().unwrap().speed_mps = f64::NAN,
            1 => invalid.units[0].hit_points = 0,
            2 => invalid.mission.as_mut().unwrap().target_unit_ids = vec![SHOOTER],
            3 => invalid.mission.as_mut().unwrap().target_unit_ids = vec![ENEMY, ENEMY],
            4 => invalid.units[0].weapon.as_mut().unwrap().damage = 0,
            _ => invalid.mission.as_mut().unwrap().deadline_tick = 0,
        }
        assert!(invalid.validate(&sides).is_err());
    }
}

#[test]
fn truth_inspection_reports_pending_impact_without_a_fabricated_flight_position() {
    let mut sim = fixture(2, 100, 60);
    sim.step();
    fire(&mut sim, 100, 0);
    sim.step();
    let truth = sim.truth_projection();
    assert_eq!(truth.weapons.len(), 1);
    let weapon = &truth.weapons[0];
    assert_eq!(weapon.kind, "training_pending_impact");
    assert!(weapon.position.is_none());
    assert!(weapon.aim_point.is_some());
    assert!(weapon.impact_tick.unwrap() > truth.tick);
    assert_eq!(
        truth
            .units
            .iter()
            .find(|u| u.state.id == SHOOTER)
            .unwrap()
            .ammunition,
        1
    );
}
