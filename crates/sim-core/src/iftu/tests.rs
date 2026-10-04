use super::*;
const SHOOTER: Uuid = Uuid::from_u128(1);
const TARGET: Uuid = Uuid::from_u128(2);
const SENSOR: Uuid = Uuid::from_u128(3);
const RELAY: Uuid = Uuid::from_u128(4);
const BACKUP: Uuid = Uuid::from_u128(5);
fn fixture() -> Simulation {
    let mut platforms: Vec<_> = [SHOOTER, TARGET, SENSOR, RELAY, BACKUP]
        .into_iter()
        .map(|id| PlatformSpawn {
            id,
            name: format!("Unit {id}"),
            side: if id == TARGET { Side::Red } else { Side::Blue },
            domain: Domain::Air,
            pose: GeoPose {
                latitude_deg: 0.0,
                longitude_deg: if id == TARGET { 0.25 } else { 0.0 },
                altitude_m: 5000.0,
            },
            velocity: Velocity::default(),
            sensor: (id == SENSOR).then_some(Sensor {
                range_m: 100_000.0,
                identification_range_m: 100_000.0,
                scan_interval_ticks: 1,
                field_of_regard_deg: 360.0,
            }),
            network_device_ids: vec![],
            flight_path: None,
            sidc: unknown_sidc(Side::Blue).into(),
        })
        .collect();
    let mut network = NetworkConfig::default();
    let mut links = vec![];
    let mut options = SimulatorOptions::default();
    for (i, (from, to)) in [
        (SENSOR, RELAY),
        (RELAY, SHOOTER),
        (SHOOTER, BACKUP),
        (BACKUP, SHOOTER),
        (SHOOTER, SENSOR),
    ]
    .into_iter()
    .enumerate()
    {
        let source: DeviceId = format!("tx-{i}").into();
        let sink: DeviceId = format!("rx-{i}").into();
        let channel: ChannelId = format!("link-{i}").into();
        for (id, node, kind) in [
            (
                source.clone(),
                from,
                DeviceKind::Source {
                    egress: channel.clone(),
                },
            ),
            (sink.clone(), to, DeviceKind::Sink),
        ] {
            network.devices.push(DeviceConfig {
                id: id.clone(),
                kind,
                mobility: Default::default(),
                interference: vec![],
            });
            platforms
                .iter_mut()
                .find(|p| p.id == node)
                .unwrap()
                .network_device_ids
                .push(id);
        }
        network.channels.push(ChannelConfig {
            id: channel.clone(),
            endpoints: [source.clone(), sink.clone()],
            bit_rate_bps: 64000,
            propagation_delay_ns: 100_000_000,
            state: Default::default(),
            distance: None,
            radio: Some(RadioChannel {
                band: FrequencyBand::new(960_000_000, 1_215_000_000),
                interference_response: Default::default(),
            }),
        });
        options.channels.insert(
            channel.clone(),
            ChannelOptions {
                queue: QueueConfig {
                    max_packets: Some(32),
                    max_bytes: Some(65536),
                    discipline: Default::default(),
                },
                ..Default::default()
            },
        );
        links.push(CommunicationLinkDefinition {
            id: format!("link-{i}"),
            from_entity_id: from,
            to_entity_id: to,
            source_device_id: source,
            destination_device_id: sink,
            channel_id: channel,
        });
    }
    for p in &mut platforms {
        if p.network_device_ids.is_empty() {
            let id: DeviceId = format!("endpoint-{}", p.id).into();
            network.devices.push(DeviceConfig {
                id: id.clone(),
                kind: DeviceKind::Sink,
                mobility: Default::default(),
                interference: vec![],
            });
            p.network_device_ids.push(id);
        }
    }
    let mut sim = Simulation::new(
        platforms,
        CommunicationsConfig {
            network,
            simulator_options: options,
            links,
            jamming_regions: vec![],
        },
    )
    .unwrap();
    sim.configure_iftu(Configuration {
        equipment: vec![
            Equipment {
                unit_id: SHOOTER,
                platform_fit: "Training launcher".into(),
            },
            Equipment {
                unit_id: BACKUP,
                platform_fit: "Training launcher".into(),
            },
        ],
        loadouts: vec![Loadout {
            unit_id: SHOOTER,
            weapon_id: "game-interceptor".into(),
            provider_id: SHOOTER,
            satellite_relay_id: None,
        }],
        subscriptions: vec![Subscription {
            source_id: SENSOR,
            provider_id: SHOOTER,
            interval_ticks: 1,
        }],
    })
    .unwrap();
    sim
}
fn launch(sim: &mut Simulation) -> Uuid {
    for _ in 0..3 {
        sim.step();
    }
    let track = sim.world.resource::<KnowledgeBases>().0[&SHOOTER][0].clone();
    sim.world
        .resource_mut::<WeaponSystem>()
        .launches
        .push(Launch {
            launcher: SHOOTER,
            position: GeoPose {
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                altitude_m: 5000.0,
            },
            side: Side::Blue,
            track,
        });
    sim.step();
    *sim.world
        .resource::<WeaponSystem>()
        .flights
        .keys()
        .next()
        .unwrap()
}
fn jam(sim: &mut Simulation, band: FrequencyBand, until: u64) {
    sim.communications.jamming_regions.push(JammingRegion {
        id: "jam".into(),
        name: "Test interference".into(),
        center: GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 5000.0,
        },
        radius_m: 100000.0,
        band,
        jammed: 1.0,
        active_from_tick: 0,
        active_until_tick: Some(until),
    });
}
#[test]
fn reports_route_through_relay_before_provider_translates_updates() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    for _ in 0..6 {
        sim.step();
    }
    let s = sim.world.resource::<WeaponSystem>();
    assert!(s.flights[&id].accepted_sequence > 0);
    assert!(s.trace.iter().any(|m| m.kind == "track_report"
        && m.from == SENSOR
        && m.to == RELAY
        && m.state == "delivered"));
    assert!(s.trace.iter().any(|m| m.kind == "track_report"
        && m.from == RELAY
        && m.to == SHOOTER
        && m.state == "delivered"));
    assert!(s.trace.iter().any(|m| m.kind == "weapon_target_update"
        && m.from == SHOOTER
        && m.to == id
        && m.state == "accepted"));
    assert_eq!(s.flights[&id].source, SENSOR);
}
#[test]
fn jammed_weapon_cannot_receive_updates_but_sensor_reports_arrive() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    jam(
        &mut sim,
        FrequencyBand::new(8_000_000_000, 9_000_000_000),
        12,
    );
    for _ in 0..6 {
        sim.step();
    }
    let s = sim.world.resource::<WeaponSystem>();
    assert_eq!(s.flights[&id].accepted_sequence, 0);
    assert!(s
        .trace
        .iter()
        .any(|m| m.kind == "weapon_target_update" && m.state == "dropped"));
    assert!(sim.world.resource::<KnowledgeBases>().0[&SHOOTER][0].observed_tick > 4);
    for _ in 0..6 {
        sim.step();
    }
    assert!(sim.world.resource::<WeaponSystem>().flights[&id].accepted_sequence > 0);
}
#[test]
fn jammed_upstream_reports_do_not_provide_fresh_target_positions() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    jam(
        &mut sim,
        FrequencyBand::new(960_000_000, 1_215_000_000),
        100,
    );
    for _ in 0..8 {
        sim.step();
    }
    let s = sim.world.resource::<WeaponSystem>();
    assert!(s.flights[&id].estimate.observed_tick < 6);
    assert!(s
        .trace
        .iter()
        .any(|m| m.kind == "track_report" && m.state == "dropped"));
}
#[test]
fn projections_do_not_reveal_one_way_receiver_state_or_enemy_weapons() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    for _ in 0..6 {
        sim.step();
    }
    assert!(sim.world.resource::<WeaponSystem>().flights[&id].accepted_sequence > 0);
    let p = sim.iftu_projection(SHOOTER);
    assert!(p.weapons[0].update_confirmation.starts_with("Unconfirmed"));
    assert!(!p
        .messages
        .iter()
        .any(|m| m.to == id && m.state == "accepted"));
    assert!(sim.iftu_projection(TARGET).weapons.is_empty());
}
#[test]
fn stale_duplicate_and_wrong_provider_updates_are_rejected() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    let mut s = sim.world.remove_resource::<WeaponSystem>().unwrap();
    let f = &s.flights[&id];
    let original = f.estimate.position.longitude_deg;
    let update = WeaponTargetUpdate {
        weapon_id: id,
        provider_id: BACKUP,
        assignment_revision: 1,
        sequence: 1,
        source_id: f.source,
        source_track_id: f.source_track,
        track: f.estimate.clone(),
        expires_at_ns: 100 * NS,
        retarget: false,
    };
    sim.send_iftu_datagram(&mut s, BACKUP, id, Payload::Update(update), Some(vec![]));
    assert_eq!(s.flights[&id].accepted_sequence, 0);
    assert_eq!(s.flights[&id].estimate.position.longitude_deg, original);
    assert!(s.trace.back().unwrap().detail.contains("provider"));
    sim.world.insert_resource(s);
}
#[test]
fn incompatible_equipment_is_rejected_at_configuration() {
    let mut sim = fixture();
    let mut config = sim.world.resource::<WeaponSystem>().config.clone();
    config.equipment[0].platform_fit = "E-3G".into();
    assert!(sim.configure_iftu(config).is_err());
}
#[test]
fn weapon_expiry_retires_runtime_endpoints() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    sim.world
        .resource_mut::<WeaponSystem>()
        .flights
        .get_mut(&id)
        .unwrap()
        .profile
        .lifetime_seconds = 0.5;
    sim.step();
    let s = sim.world.resource::<WeaponSystem>();
    let f = &s.flights[&id];
    assert_eq!(f.phase, "expired");
    for device in &f.device_ids {
        assert!(sim
            .communications
            .simulator
            .device_position_at(device.clone(), sim.communications.simulator.now())
            .is_err());
    }
}
#[test]
fn replay_produces_same_flight_and_message_history() {
    let mut a = fixture();
    let mut b = fixture();
    let id = launch(&mut a);
    assert_eq!(launch(&mut b), id);
    for _ in 0..10 {
        a.step();
        b.step();
    }
    let a = a.world.resource::<WeaponSystem>();
    let b = b.world.resource::<WeaponSystem>();
    assert_eq!(
        a.flights[&id].position.longitude_deg,
        b.flights[&id].position.longitude_deg
    );
    assert_eq!(
        serde_json::to_string(&a.trace).unwrap(),
        serde_json::to_string(&b.trace).unwrap()
    );
}

#[test]
fn handoff_does_not_change_receiver_authority_until_assignment_arrives() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    sim.world
        .resource_mut::<WeaponSystem>()
        .flights
        .get_mut(&id)
        .unwrap()
        .profile
        .handoff = true;
    jam(
        &mut sim,
        FrequencyBand::new(8_000_000_000, 9_000_000_000),
        100,
    );
    sim.world.resource_mut::<WeaponSystem>().commands.push((
        SHOOTER,
        id,
        Command::AssignProvider {
            provider_id: BACKUP,
        },
    ));
    for _ in 0..3 {
        sim.step();
    }
    let s = sim.world.resource::<WeaponSystem>();
    assert_eq!(s.flights[&id].provider, SHOOTER);
    assert_eq!(s.flights[&id].sender_provider, BACKUP);
    sim.communications.jamming_regions.clear();
    sim.world.resource_mut::<WeaponSystem>().commands.push((
        SHOOTER,
        id,
        Command::AssignProvider {
            provider_id: BACKUP,
        },
    ));
    for _ in 0..3 {
        sim.step();
    }
    let s = sim.world.resource::<WeaponSystem>();
    assert_eq!(s.flights[&id].provider, BACKUP);
    assert!(s.flights[&id].revision > 1);
    // A one-way link cannot reveal receiver-side handoff acceptance to the launcher.
    assert_eq!(sim.iftu_projection(SHOOTER).weapons[0].provider_id, SHOOTER);
}
#[test]
fn status_confirmation_requires_the_return_packet_and_preserves_observation_time() {
    let mut sim = fixture();
    sim.world
        .resource_mut::<WeaponSystem>()
        .catalog
        .weapons
        .iter_mut()
        .find(|p| p.id == "game-interceptor")
        .unwrap()
        .telemetry = true;
    let id = launch(&mut sim);
    for _ in 0..5 {
        sim.step();
    }
    let s = sim.world.resource::<WeaponSystem>();
    let f = &s.flights[&id];
    assert!(f.status.contains_key(&SHOOTER));
    assert!(f.status[&SHOOTER].2 < s.clock_ns);
    assert!(sim.iftu_projection(SHOOTER).weapons[0]
        .update_confirmation
        .contains("confirmed"));
    let observed = f.status[&SHOOTER].2;
    jam(
        &mut sim,
        FrequencyBand::new(8_000_000_000, 9_000_000_000),
        100,
    );
    for _ in 0..4 {
        sim.step();
    }
    assert_eq!(
        sim.world.resource::<WeaponSystem>().flights[&id].status[&SHOOTER].2,
        observed
    );
}
#[test]
fn stale_duplicate_and_post_terminal_packets_cannot_change_the_estimate() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    let mut s = sim.world.remove_resource::<WeaponSystem>().unwrap();
    let f = &s.flights[&id];
    let mut track = f.estimate.clone();
    track.observed_tick = s.clock_ns / NS;
    let original = track.position.longitude_deg;
    track.position.longitude_deg += 0.01;
    let update = WeaponTargetUpdate {
        weapon_id: id,
        provider_id: SHOOTER,
        assignment_revision: 1,
        sequence: 1,
        source_id: f.source,
        source_track_id: f.source_track,
        track: track.clone(),
        expires_at_ns: s.clock_ns + 15 * NS,
        retarget: false,
    };
    sim.send_iftu_datagram(
        &mut s,
        SHOOTER,
        id,
        Payload::Update(update.clone()),
        Some(vec![]),
    );
    assert_eq!(s.flights[&id].accepted_sequence, 1);
    let accepted = s.flights[&id].estimate.position.longitude_deg;
    assert_ne!(accepted, original);
    let mut duplicate = update.clone();
    duplicate.track.position.longitude_deg += 0.1;
    sim.send_iftu_datagram(
        &mut s,
        SHOOTER,
        id,
        Payload::Update(duplicate),
        Some(vec![]),
    );
    assert_eq!(s.flights[&id].estimate.position.longitude_deg, accepted);
    let mut stale = update.clone();
    stale.sequence = 2;
    stale.expires_at_ns = s.clock_ns;
    sim.send_iftu_datagram(&mut s, SHOOTER, id, Payload::Update(stale), Some(vec![]));
    assert_eq!(s.flights[&id].accepted_sequence, 1);
    s.flights.get_mut(&id).unwrap().phase = "impact".into();
    let mut terminal = update;
    terminal.sequence = 3;
    terminal.retarget = true;
    sim.send_iftu_datagram(&mut s, SHOOTER, id, Payload::Update(terminal), Some(vec![]));
    assert_eq!(s.flights[&id].accepted_sequence, 1);
    sim.world.insert_resource(s);
}
#[test]
fn retargeting_requires_a_capable_profile_and_delivery() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    let mut s = sim.world.remove_resource::<WeaponSystem>().unwrap();
    let f = &s.flights[&id];
    let mut track = f.estimate.clone();
    track.track_id = Uuid::from_u128(99);
    track.observed_tick = s.clock_ns / NS;
    track.position.longitude_deg = 0.4;
    let update = WeaponTargetUpdate {
        weapon_id: id,
        provider_id: SHOOTER,
        assignment_revision: 1,
        sequence: 1,
        source_id: SENSOR,
        source_track_id: track.track_id,
        track: track.clone(),
        expires_at_ns: s.clock_ns + 15 * NS,
        retarget: true,
    };
    sim.send_iftu_datagram(
        &mut s,
        SHOOTER,
        id,
        Payload::Update(update.clone()),
        Some(vec![]),
    );
    assert_ne!(s.flights[&id].estimate.position.longitude_deg, 0.4);
    s.flights.get_mut(&id).unwrap().profile.retarget = true;
    let route = s.flights[&id].links[&SHOOTER].clone();
    sim.send_iftu_datagram(&mut s, SHOOTER, id, Payload::Update(update), Some(route));
    assert_ne!(s.flights[&id].estimate.position.longitude_deg, 0.4);
    sim.world.insert_resource(s);
    sim.step();
    assert_eq!(
        sim.world.resource::<WeaponSystem>().flights[&id]
            .estimate
            .position
            .longitude_deg,
        0.4
    );
}
#[test]
fn seeker_observes_locally_after_update_loss_and_misses_outside_acquisition() {
    let mut sim = fixture();
    let id = launch(&mut sim);
    jam(
        &mut sim,
        FrequencyBand::new(8_000_000_000, 9_000_000_000),
        100,
    );
    sim.world
        .resource_mut::<WeaponSystem>()
        .flights
        .get_mut(&id)
        .unwrap()
        .position = GeoPose {
        latitude_deg: 0.0,
        longitude_deg: 0.23,
        altitude_m: 5000.0,
    };
    sim.step();
    assert_eq!(
        sim.world.resource::<WeaponSystem>().flights[&id].phase,
        "terminal"
    );
    let mut other = fixture();
    let oid = launch(&mut other);
    jam(
        &mut other,
        FrequencyBand::new(8_000_000_000, 9_000_000_000),
        100,
    );
    let mut q = other
        .world
        .query::<(&SimEntityId, &mut GeoPose, &mut GeodesicMotionState)>();
    for (id, mut p, mut motion) in q.iter_mut(&mut other.world) {
        if id.0 == TARGET {
            p.latitude_deg = 1.0;
            motion.origin = *p;
        }
    }
    for _ in 0..40 {
        other.step();
    }
    assert_eq!(
        other.world.resource::<WeaponSystem>().flights[&oid].phase,
        "impact"
    );
    assert!(!other.weapon_positions()[&TARGET].2);
}
