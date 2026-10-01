use super::*;

fn platform(
    id: u128,
    side: Side,
    longitude_deg: f64,
    altitude_m: f64,
    sensor: Option<Sensor>,
) -> PlatformSpawn {
    PlatformSpawn {
        id: Uuid::from_u128(id),
        name: format!("Sensor fixture {id}"),
        side,
        domain: Domain::Air,
        pose: GeoPose {
            latitude_deg: 0.0,
            longitude_deg,
            altitude_m,
        },
        velocity: Velocity::default(),
        sensor,
        network_device_ids: vec![DeviceId::new(format!("sensor-fixture-{id}"))],
        flight_path: None,
        sidc: unknown_sidc(side).into(),
    }
}

fn communications(platforms: &[PlatformSpawn]) -> CommunicationsConfig {
    CommunicationsConfig {
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
    }
}

fn simulation(platforms: Vec<PlatformSpawn>) -> Simulation {
    Simulation::new(platforms.clone(), communications(&platforms)).unwrap()
}

fn set_position(sim: &mut Simulation, id: u128, longitude_deg: f64, altitude_m: f64) {
    let mut query = sim.world.query::<(&SimEntityId, &mut GeoPose)>();
    let (_, mut pose) = query
        .iter_mut(&mut sim.world)
        .find(|(unit_id, _)| unit_id.0 == Uuid::from_u128(id))
        .unwrap();
    pose.longitude_deg = longitude_deg;
    pose.altitude_m = altitude_m;
}

#[test]
fn detects_unit_when_inside_radar_horizon_and_blocks_a_low_target_beyond_it() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            20.0,
            Some(Sensor {
                range_m: 200_000.0,
                identification_range_m: 30_000.0,
            }),
        ),
        platform(2, Side::Red, 0.01, 20.0, None),
        platform(3, Side::Red, 1.0, 20.0, None),
        platform(4, Side::Blue, 0.01, 20.0, None),
    ]);
    sim.step();
    let projection = sim.projection_for(Uuid::from_u128(1), Side::Blue);
    assert_eq!(projection.tracks.len(), 1);
    assert_eq!(
        projection.tracks[0].track_id,
        scoped_track_id(Uuid::nil(), Uuid::from_u128(1), Uuid::from_u128(2))
    );
    assert_eq!(projection.tracks[0].identity_confidence, 0.9);
    set_position(&mut sim, 3, 1.0, 1_500.0);
    sim.step();
    let projection = sim.projection_for(Uuid::from_u128(1), Side::Blue);
    assert_eq!(projection.tracks.len(), 2);
    assert_eq!(
        projection
            .tracks
            .iter()
            .find(|track| track.track_id
                == scoped_track_id(Uuid::nil(), Uuid::from_u128(1), Uuid::from_u128(3)))
            .unwrap()
            .identity_confidence,
        0.45
    );
}

#[test]
fn sensor_range_and_identification_use_altitude_separation() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 15_000.0,
                identification_range_m: 5_000.0,
            }),
        ),
        platform(2, Side::Red, 0.0, 21_000.0, None),
    ]);
    sim.step();
    assert!(sim
        .projection_for(Uuid::from_u128(1), Side::Blue)
        .tracks
        .is_empty());
    set_position(&mut sim, 2, 0.0, 10_000.0);
    sim.step();
    assert_eq!(
        sim.projection_for(Uuid::from_u128(1), Side::Blue).tracks[0].identity_confidence,
        0.45
    );
    set_position(&mut sim, 2, 0.0, 5_000.0);
    sim.step();
    assert_eq!(
        sim.projection_for(Uuid::from_u128(1), Side::Blue).tracks[0].identity_confidence,
        0.9
    );
}

#[test]
fn lost_contact_retains_last_observation_without_following_hidden_truth() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            20.0,
            Some(Sensor {
                range_m: 200_000.0,
                identification_range_m: 30_000.0,
            }),
        ),
        platform(2, Side::Red, 0.1, 1_000.0, None),
    ]);
    sim.step();
    set_position(&mut sim, 2, 1.0, 0.0);
    sim.step();
    let projection = sim.projection_for(Uuid::from_u128(1), Side::Blue);
    assert_eq!(projection.tick, 2);
    let track = &projection.tracks[0];
    assert_eq!(track.position.longitude_deg, 0.1);
    assert_eq!(track.position.altitude_m, 1_000.0);
    assert_eq!(track.observed_tick, 1);
    assert_eq!(track.received_tick, 1);
    set_position(&mut sim, 2, 1.0, 5_000.0);
    sim.step();
    let projection = sim.projection_for(Uuid::from_u128(1), Side::Blue);
    assert_eq!(projection.tracks.len(), 1);
    assert_eq!(projection.tracks[0].position.longitude_deg, 1.0);
    assert_eq!(projection.tracks[0].observed_tick, 3);
}

#[test]
fn local_detection_does_not_grant_another_terminal_the_same_tracks() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
        ),
        platform(2, Side::Red, 0.01, 1_000.0, None),
        platform(3, Side::Blue, 0.0, 1_000.0, None),
    ]);
    sim.step();
    assert_eq!(
        sim.projection_for(Uuid::from_u128(1), Side::Blue)
            .tracks
            .len(),
        1
    );
    assert!(sim
        .projection_for(Uuid::from_u128(3), Side::Blue)
        .tracks
        .is_empty());
}

#[test]
fn invalid_sensor_ranges_are_rejected_when_building_a_simulation() {
    for sensor in [
        Sensor {
            range_m: f64::NAN,
            identification_range_m: 1.0,
        },
        Sensor {
            range_m: 0.0,
            identification_range_m: 0.0,
        },
        Sensor {
            range_m: 10.0,
            identification_range_m: -1.0,
        },
        Sensor {
            range_m: 10.0,
            identification_range_m: 11.0,
        },
        Sensor {
            range_m: 10.0,
            identification_range_m: f64::INFINITY,
        },
    ] {
        let platforms = vec![platform(1, Side::Blue, 0.0, 1_000.0, Some(sensor))];
        let error = Simulation::validate_configuration(&platforms, &communications(&platforms))
            .unwrap_err();
        assert!(error.to_string().contains("invalid sensor ranges"));
    }
}

#[test]
fn invalid_initial_positions_and_velocities_are_rejected() {
    for (longitude_deg, north_mps) in [(181.0, 0.0), (f64::NAN, 0.0), (0.0, f64::INFINITY)] {
        let mut unit = platform(1, Side::Blue, longitude_deg, 1_000.0, None);
        unit.velocity.north_mps = north_mps;
        let platforms = vec![unit];
        let error = Simulation::validate_configuration(&platforms, &communications(&platforms))
            .unwrap_err();
        assert!(error.to_string().contains("invalid position or velocity"));
    }
}

#[test]
fn track_identity_is_scoped_to_the_terminal_and_game_and_stable_within_a_replay() {
    let platforms = vec![
        platform(
            1,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
        ),
        platform(2, Side::Red, 0.01, 1_000.0, None),
        platform(
            3,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
        ),
    ];
    let run = |namespace| {
        let mut sim = Simulation::new_with_knowledge_namespace(
            platforms.clone(),
            communications(&platforms),
            namespace,
        )
        .unwrap();
        sim.step();
        let first = sim.projection_for(Uuid::from_u128(1), Side::Blue).tracks[0].track_id;
        let second = sim.projection_for(Uuid::from_u128(3), Side::Blue).tracks[0].track_id;
        assert_ne!(first, Uuid::from_u128(2));
        assert_ne!(first, second);
        sim.step();
        assert_eq!(
            sim.projection_for(Uuid::from_u128(1), Side::Blue).tracks[0].track_id,
            first
        );
        (first, second)
    };
    let ids = run(Uuid::from_u128(900));
    assert_eq!(ids, run(Uuid::from_u128(900)));
    assert_ne!(ids, run(Uuid::from_u128(901)));
}

#[test]
fn delayed_report_uses_its_measurement_and_keeps_observation_and_receipt_times_separate() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
        ),
        platform(2, Side::Red, 0.01, 1_000.0, None),
        platform(3, Side::Blue, 0.0, 1_000.0, None),
    ]);
    sim.step();
    let original = sim.current_sensor_tracks(Uuid::from_u128(1))[0].clone();
    set_position(&mut sim, 2, 5.0, 1_000.0);
    sim.step();
    sim.step();
    assert!(sim.current_sensor_tracks(Uuid::from_u128(1)).is_empty());
    assert!(sim.receive_track_report(Uuid::from_u128(3), original.clone()));
    let received = sim.projection_for(Uuid::from_u128(3), Side::Blue).tracks;
    assert_eq!(received.len(), 1);
    assert_eq!(
        received[0].position.longitude_deg,
        original.position.longitude_deg
    );
    assert_eq!(received[0].observed_tick, 1);
    assert_eq!(received[0].received_tick, 3);
    assert_ne!(received[0].track_id, original.track_id);
    assert!(sim.current_sensor_tracks(Uuid::from_u128(3)).is_empty());
}

#[test]
fn duplicate_or_older_source_report_cannot_overwrite_a_newer_report() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
        ),
        platform(2, Side::Red, 0.01, 1_000.0, None),
        platform(3, Side::Blue, 0.0, 1_000.0, None),
    ]);
    sim.step();
    let old = sim.current_sensor_tracks(Uuid::from_u128(1))[0].clone();
    set_position(&mut sim, 2, 0.02, 1_000.0);
    sim.step();
    let newer = sim.current_sensor_tracks(Uuid::from_u128(1))[0].clone();
    assert!(sim.receive_track_report(Uuid::from_u128(3), newer.clone()));
    sim.step();
    assert!(!sim.receive_track_report(Uuid::from_u128(3), old));
    assert!(!sim.receive_track_report(Uuid::from_u128(3), newer));
    let received = sim.projection_for(Uuid::from_u128(3), Side::Blue).tracks;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].position.longitude_deg, 0.02);
    assert_eq!(received[0].observed_tick, 2);
    assert_eq!(received[0].received_tick, 2);
}

#[test]
fn report_ingestion_rejects_future_invalid_friendly_and_unknown_recipient_data() {
    let mut sim = simulation(vec![
        platform(
            1,
            Side::Blue,
            0.0,
            1_000.0,
            Some(Sensor {
                range_m: 20_000.0,
                identification_range_m: 10_000.0,
            }),
        ),
        platform(2, Side::Red, 0.01, 1_000.0, None),
        platform(3, Side::Blue, 0.0, 1_000.0, None),
    ]);
    sim.step();
    let original = sim.current_sensor_tracks(Uuid::from_u128(1))[0].clone();
    let mut future = original.clone();
    future.observed_tick += 1;
    let mut invalid_position = original.clone();
    invalid_position.position.latitude_deg = f64::NAN;
    let mut invalid_confidence = original.clone();
    invalid_confidence.identity_confidence = 1.5;
    let mut friendly = original.clone();
    friendly.target_side = Side::Blue;
    for report in [future, invalid_position, invalid_confidence, friendly] {
        assert!(!sim.receive_track_report(Uuid::from_u128(3), report));
    }
    assert!(!sim.receive_track_report(Uuid::from_u128(99), original));
    assert!(sim
        .projection_for(Uuid::from_u128(3), Side::Blue)
        .tracks
        .is_empty());
}
