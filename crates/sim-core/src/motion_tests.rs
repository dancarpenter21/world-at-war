use super::*;

const UNIT: Uuid = Uuid::from_u128(1);

fn platform(longitude_deg: f64, north_mps: f64, east_mps: f64) -> PlatformSpawn {
    PlatformSpawn {
        id: UNIT,
        name: "Motion fixture".into(),
        side: Side::Blue,
        domain: Domain::Air,
        pose: GeoPose {
            latitude_deg: 0.0,
            longitude_deg,
            altitude_m: 1_000.0,
        },
        velocity: Velocity {
            north_mps,
            east_mps,
            climb_mps: 0.0,
        },
        sensor: None,
        network_device_ids: vec![DeviceId::new("motion-fixture")],
        flight_path: None,
        sidc: unknown_sidc(Side::Blue).into(),
    }
}

fn simulation(platforms: Vec<PlatformSpawn>) -> Simulation {
    let devices = platforms
        .iter()
        .flat_map(|platform| &platform.network_device_ids)
        .map(|id| c3mesh::DeviceConfig {
            id: id.clone(),
            kind: DeviceKind::Sink,
            mobility: Default::default(),
            interference: vec![],
        })
        .collect();
    Simulation::new(
        platforms,
        CommunicationsConfig {
            network: NetworkConfig {
                devices,
                channels: vec![],
            },
            simulator_options: SimulatorOptions::default(),
            links: vec![],
            jamming_regions: vec![],
        },
    )
    .unwrap()
}

fn pose(sim: &mut Simulation) -> GeoPose {
    sim.projection_for(UNIT, Side::Blue).own_units[0].position
}

fn order(sim: &mut Simulation, id: u128, north_mps: f64, east_mps: f64) {
    sim.queue_authorized_intent(AuthorizedIntent {
        intent: PlayerIntent {
            intent_id: Uuid::from_u128(id),
            issuer_role: Uuid::from_u128(10),
            target: UNIT,
            kind: OrderKind::Move {
                north_mps,
                east_mps,
            },
            requested_tick: sim.tick(),
        },
        authorization: AuthorizationRecord {
            policy_id: Uuid::from_u128(20),
            policy_version: 1,
            requester_role_id: Uuid::from_u128(10),
            granting_role_id: Uuid::from_u128(10),
            request_id: None,
        },
    });
}

#[test]
fn dateline_crossing_preserves_a_valid_position_and_local_detection() {
    let mut blue = platform(179.999, 0.0, 500.0);
    blue.sensor = Some(Sensor {
        range_m: 1_000.0,
        identification_range_m: 500.0,
    });
    let mut red = platform(-179.999, 0.0, 0.0);
    red.id = Uuid::from_u128(2);
    red.side = Side::Red;
    red.network_device_ids = vec![DeviceId::new("red-motion-fixture")];
    let mut sim = simulation(vec![blue, red]);
    sim.step();
    assert!(pose(&mut sim).longitude_deg < -179.99);
    assert!(geo_pose_is_finite(pose(&mut sim)));
    assert_eq!(sim.projection_for(UNIT, Side::Blue).tracks.len(), 1);
}

#[test]
fn motion_continues_after_pole_crossing_instead_of_bouncing_toward_the_pole() {
    let mut unit = platform(20.0, 300.0, 0.0);
    unit.pose.latitude_deg = 89.999;
    let mut sim = simulation(vec![unit]);
    sim.step();
    let first = pose(&mut sim);
    sim.step();
    let second = pose(&mut sim);
    assert!(first.latitude_deg < 89.999);
    assert!(second.latitude_deg < first.latitude_deg);
    assert!((second.longitude_deg + 160.0).abs() < 1.0e-7);
    assert!(geo_pose_is_finite(second));
    assert_eq!(
        sim.projection_for(UNIT, Side::Blue).own_units[0]
            .velocity
            .north_mps,
        300.0
    );
}

#[test]
fn a_new_course_starts_at_the_current_position_and_stop_preserves_it() {
    let mut sim = simulation(vec![platform(179.999, 0.0, 300.0)]);
    for _ in 0..5 {
        sim.step();
    }
    let before_turn = pose(&mut sim);
    order(&mut sim, 30, 300.0, 0.0);
    sim.step();
    let after_turn = pose(&mut sim);
    assert!(after_turn.latitude_deg > before_turn.latitude_deg);
    assert!((after_turn.longitude_deg - before_turn.longitude_deg).abs() < 1.0e-9);
    assert!(matches!(
        sim.drain_order_results()[0].status,
        OrderStatus::Accepted
    ));
    order(&mut sim, 31, 0.0, 0.0);
    sim.step();
    let stopped = pose(&mut sim);
    assert_eq!(stopped.latitude_deg, after_turn.latitude_deg);
    assert_eq!(stopped.longitude_deg, after_turn.longitude_deg);
    for _ in 0..10 {
        sim.step();
    }
    let later = pose(&mut sim);
    assert_eq!(later.latitude_deg, stopped.latitude_deg);
    assert_eq!(later.longitude_deg, stopped.longitude_deg);
}

#[test]
fn cyclic_route_uses_the_short_dateline_arc_in_both_directions() {
    let mut unit = platform(179.0, 0.0, 0.0);
    unit.flight_path = Some(CyclicFlightPath {
        period_ticks: 20,
        waypoints: vec![
            FlightWaypoint {
                at_tick: 0,
                position: unit.pose,
            },
            FlightWaypoint {
                at_tick: 10,
                position: GeoPose {
                    longitude_deg: -179.0,
                    ..unit.pose
                },
            },
        ],
    });
    let mut sim = simulation(vec![unit]);
    for tick in 1..=20 {
        sim.step();
        let position = pose(&mut sim);
        assert!(position.longitude_deg.abs() >= 179.0 - 1.0e-9);
        assert!(geo_pose_is_finite(position));
        if tick == 5 || tick == 15 {
            assert!((position.longitude_deg.abs() - 180.0).abs() < 1.0e-9);
        }
    }
}

#[test]
fn nonfinite_authorized_movement_does_not_corrupt_velocity_or_cancel_a_route() {
    let mut unit = platform(0.0, 0.0, 0.0);
    unit.flight_path = Some(CyclicFlightPath {
        period_ticks: 20,
        waypoints: vec![
            FlightWaypoint {
                at_tick: 0,
                position: unit.pose,
            },
            FlightWaypoint {
                at_tick: 10,
                position: GeoPose {
                    longitude_deg: 1.0,
                    ..unit.pose
                },
            },
        ],
    });
    let mut sim = simulation(vec![unit]);
    for (id, north, east) in [
        (40, f64::INFINITY, 0.0),
        (41, 0.0, f64::NAN),
        (42, f64::MAX, f64::MAX),
    ] {
        order(&mut sim, id, north, east);
    }
    sim.step();
    let results = sim.drain_order_results();
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| matches!(&result.status, OrderStatus::Rejected(reason) if reason.contains("finite speed"))));
    let projection = sim.projection_for(UNIT, Side::Blue);
    assert!(projection.own_units[0].following_flight_path);
    assert_eq!(projection.own_units[0].velocity, Velocity::default());
    assert!(geo_pose_is_finite(projection.own_units[0].position));
}
