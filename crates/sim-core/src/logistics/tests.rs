use super::*;
use c3mesh::{DeviceConfig, SimulatorOptions};
fn configuration() -> Configuration {
    serde_json::from_str(include_str!(
        "../../../../data/logistics/joint-exercise.v1.json"
    ))
    .unwrap()
}
fn fixture(mut config: Configuration) -> Simulation {
    // A smaller deterministic inventory fixture, independent of playable depot tuning.
    for facility in &mut config.facilities {
        if facility.unit_id == Uuid::from_u128(5) {
            facility.stock.fuel_kg = 400.0;
        }
        if facility.unit_id == Uuid::from_u128(25) {
            facility.stock.fuel_kg = 40.0;
        }
    }
    if let Some(cargo) = config
        .units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(14))
    {
        cargo.fuel_kg = 600.0;
    }

    // Constant consumption isolates exact transfer and conservation expectations.
    for u in &mut config.units {
        u.profile.burn.kg_per_second.fill(1.0);
        u.profile.idle_burn.kg_per_second.fill(1.0);
        u.profile.climb_burn.kg_per_second.fill(1.0);
        u.profile.combat_burn.kg_per_second.fill(1.0);
    }
    let ids: BTreeSet<_> = config
        .units
        .iter()
        .map(|u| u.unit_id)
        .chain(config.facilities.iter().map(|f| f.unit_id))
        .collect();
    let platforms: Vec<_> = ids
        .into_iter()
        .map(|id| PlatformSpawn {
            id,
            name: "Fictional logistics fixture".into(),
            side: Side::Blue,
            domain: if [11, 12, 13, 14].contains(&id.as_u128()) {
                Domain::Air
            } else {
                Domain::Sea
            },
            pose: GeoPose {
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                altitude_m: 0.0,
            },
            velocity: Velocity::default(),
            sensor: None,
            network_device_ids: vec![DeviceId::new(id.to_string())],
            flight_path: None,
            sidc: unknown_sidc(Side::Blue).into(),
        })
        .collect();
    let communications = CommunicationsConfig {
        network: NetworkConfig {
            devices: platforms
                .iter()
                .flat_map(|p| &p.network_device_ids)
                .map(|id| DeviceConfig {
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
    sim.configure_logistics(config).unwrap();
    sim
}
fn command(sim: &mut Simulation, id: u128, c: Command) {
    let tick = sim.tick();
    sim.world
        .resource_mut::<Runtime>()
        .command(Uuid::from_u128(id), &c, tick)
        .unwrap();
}
fn status(sim: &Simulation, id: u128) -> UnitStatus {
    sim.logistics_projection(Uuid::from_u128(id)).unwrap().units[0].clone()
}
fn total(sim: &Simulation) -> f64 {
    let r = sim.world.resource::<Runtime>();
    r.units
        .values()
        .map(|u| {
            u.status.fuel_kg
                + u.status.cargo.fuel_kg
                + u.cargo_task.as_ref().map_or(0.0, |(_, _, s)| s.fuel_kg)
        })
        .sum::<f64>()
        + r.facilities.values().map(|f| f.stock.fuel_kg).sum::<f64>()
}
#[test]
fn interpolates_performance_tables_without_extrapolating() {
    let p = configuration().units[0].profile.clone();
    assert_eq!(p.burn.rate(0.0, 0.0, 1000.0), Some(1.0));
    assert!((p.burn.rate(7500.0, 175.0, 1750.0).unwrap() - 1.85).abs() < 1e-10);
    assert!(p.burn.rate(20000.0, 100.0, 1500.0).is_none());
}
#[test]
fn rejects_nan_tables_rates_and_unproven_values() {
    let mut c = configuration();
    c.units[0].profile.burn.kg_per_second[0] = f64::NAN;
    let units = c
        .units
        .iter()
        .map(|u| (u.unit_id, (Side::Blue, Domain::Air)))
        .chain(
            c.facilities
                .iter()
                .map(|f| (f.unit_id, (Side::Blue, Domain::Sea))),
        )
        .collect();
    assert!(c.validate(&units).is_err());
    let mut c = configuration();
    c.units[0].profile.evidence.source.clear();
    assert!(c.validate(&units).is_err());
}
#[test]
fn transfers_at_receiver_rate_and_preserves_donor_reserve() {
    let mut c = configuration();
    c.units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(13))
        .unwrap()
        .fuel_kg = 80.0;
    let mut sim = fixture(c);
    command(
        &mut sim,
        13,
        Command::Support {
            interface: Interface::Boom,
        },
    );
    command(
        &mut sim,
        11,
        Command::Refuel {
            donor_id: Uuid::from_u128(13),
            interface: Interface::Boom,
            amount_kg: 100.0,
        },
    );
    for _ in 0..5 {
        sim.step();
    }
    let r = status(&sim, 11);
    assert_eq!(r.transferred_kg, 25.0);
    assert_eq!(status(&sim, 13).fuel_kg, 50.0);
    assert_eq!(r.phase, Phase::Disconnecting);
}
#[test]
fn fuel_transfer_requires_delivered_compatible_support() {
    let mut sim = fixture(configuration());
    let tick = sim.tick();
    assert!(sim
        .world
        .resource_mut::<Runtime>()
        .command(
            Uuid::from_u128(11),
            &Command::Refuel {
                donor_id: Uuid::from_u128(13),
                interface: Interface::Boom,
                amount_kg: 100.0
            },
            tick
        )
        .is_err());
    command(
        &mut sim,
        13,
        Command::Support {
            interface: Interface::Drogue,
        },
    );
    assert!(sim
        .world
        .resource_mut::<Runtime>()
        .command(
            Uuid::from_u128(11),
            &Command::Refuel {
                donor_id: Uuid::from_u128(13),
                interface: Interface::Drogue,
                amount_kg: 100.0
            },
            tick
        )
        .is_err());
}
#[test]
fn cargo_reservation_cancellation_and_duplicate_failure_conserve_stock() {
    let mut sim = fixture(configuration());
    let before = total(&sim);
    let task = Command::Cargo {
        source_id: Uuid::from_u128(5),
        destination_id: Uuid::from_u128(25),
        manifest: Stock {
            fuel_kg: 100.0,
            ..Default::default()
        },
    };
    command(&mut sim, 14, task.clone());
    assert_eq!(total(&sim), before);
    let tick = sim.tick();
    assert!(sim
        .world
        .resource_mut::<Runtime>()
        .command(Uuid::from_u128(14), &task, tick)
        .is_err());
    command(&mut sim, 14, Command::Cancel);
    assert_eq!(total(&sim), before);
}
#[test]
fn cargo_arrives_only_after_loading_transport_and_unloading() {
    let mut sim = fixture(configuration());
    command(
        &mut sim,
        14,
        Command::Cargo {
            source_id: Uuid::from_u128(5),
            destination_id: Uuid::from_u128(25),
            manifest: Stock {
                fuel_kg: 100.0,
                ..Default::default()
            },
        },
    );
    assert_eq!(
        sim.world.resource::<Runtime>().facilities[&Uuid::from_u128(25)]
            .stock
            .fuel_kg,
        40.0
    );
    for _ in 0..15 {
        sim.step();
    }
    assert!(
        sim.world.resource::<Runtime>().facilities[&Uuid::from_u128(25)]
            .stock
            .fuel_kg
            > 40.0
    );
}
#[test]
fn save_resume_preserves_partial_transfer_and_report_knowledge() {
    let mut sim = fixture(configuration());
    command(
        &mut sim,
        13,
        Command::Support {
            interface: Interface::Boom,
        },
    );
    command(
        &mut sim,
        11,
        Command::Refuel {
            donor_id: Uuid::from_u128(13),
            interface: Interface::Boom,
            amount_kg: 100.0,
        },
    );
    for _ in 0..4 {
        sim.step();
    }
    let mut restored = Simulation::restore(sim.snapshot()).unwrap();
    for _ in 0..5 {
        sim.step();
        restored.step();
    }
    assert_eq!(
        serde_json::to_value(sim.logistics_truth()).unwrap(),
        serde_json::to_value(restored.logistics_truth()).unwrap()
    );
}
#[test]
fn remote_stock_is_unknown_until_report_receipt_and_older_reports_cannot_overwrite() {
    let mut sim = fixture(configuration());
    let owner = Uuid::from_u128(5);
    let remote = Uuid::from_u128(25);
    assert_eq!(sim.logistics_projection(owner).unwrap().facilities.len(), 1);
    let old = sim.local_logistics_report(remote).unwrap();
    sim.step();
    let report = sim.local_logistics_report(remote).unwrap();
    assert!(sim.receive_logistics_report(owner, remote, report));
    assert!(!sim.receive_logistics_report(owner, remote, old));
    assert_eq!(sim.logistics_projection(owner).unwrap().facilities.len(), 2);
}
#[test]
fn reserve_returns_and_zero_fuel_has_domain_specific_effects() {
    let mut c = configuration();
    c.units[0].fuel_kg = 49.0;
    c.units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(15))
        .unwrap()
        .fuel_kg = 0.5;
    let mut sim = fixture(c);
    sim.step();
    assert!(matches!(
        status(&sim, 11).phase,
        Phase::Returning | Phase::Approach
    ));
    assert_eq!(status(&sim, 15).phase, Phase::Immobilized);
}
#[test]
fn moving_carrier_keeps_embarked_aircraft_attached_and_blocks_tactical_movement() {
    let mut c = configuration();
    let aircraft = c
        .units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(12))
        .unwrap();
    aircraft.initial_facility = Some(Uuid::from_u128(27));
    let mut sim = fixture(c);
    let mut q = sim.world.query::<(&SimEntityId, &mut Velocity)>();
    for (id, mut v) in q.iter_mut(&mut sim.world) {
        if id.0 == Uuid::from_u128(27) {
            v.east_mps = 10.0;
        }
    }
    assert!(!sim
        .world
        .resource::<Runtime>()
        .movement_allowed(Uuid::from_u128(12)));
    for _ in 0..5 {
        sim.step();
    }
    let truth = sim.truth_projection();
    let aircraft = truth
        .units
        .iter()
        .find(|u| u.state.id == Uuid::from_u128(12))
        .unwrap();
    let carrier = truth
        .units
        .iter()
        .find(|u| u.state.id == Uuid::from_u128(27))
        .unwrap();
    assert_eq!(aircraft.state.position, carrier.state.position);
}
#[test]
fn underway_transfer_uses_supply_inventory_not_ship_propulsion_fuel() {
    let mut sim = fixture(configuration());
    command(
        &mut sim,
        16,
        Command::Support {
            interface: Interface::Underway,
        },
    );
    command(
        &mut sim,
        15,
        Command::Refuel {
            donor_id: Uuid::from_u128(16),
            interface: Interface::Underway,
            amount_kg: 20.0,
        },
    );
    for _ in 0..5 {
        sim.step();
    }
    assert_eq!(status(&sim, 16).fuel_kg, 895.0);
    assert_eq!(
        sim.world.resource::<Runtime>().facilities[&Uuid::from_u128(16)]
            .stock
            .fuel_kg,
        480.0
    );
    assert_eq!(status(&sim, 15).transferred_kg, 20.0);
}
#[test]
fn an_incompatible_side_cannot_reserve_or_receive_resources() {
    let mut sim = fixture(configuration());
    sim.world
        .resource_mut::<Runtime>()
        .sides
        .insert(Uuid::from_u128(25), Side::Red);
    let result = sim.world.resource_mut::<Runtime>().command(
        Uuid::from_u128(14),
        &Command::Cargo {
            source_id: Uuid::from_u128(5),
            destination_id: Uuid::from_u128(25),
            manifest: Stock {
                fuel_kg: 20.0,
                ..Default::default()
            },
        },
        0,
    );
    assert!(result.is_err());
}
#[test]
fn fractional_frames_burn_once_and_service_ammunition_without_rounding_loss() {
    let mut c = configuration();
    let u = c
        .units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(11))
        .unwrap();
    u.initial_facility = Some(Uuid::from_u128(5));
    u.fuel_kg = 1000.0;
    u.ammunition = 0;
    let mut sim = fixture(c);
    let start = status(&sim, 13).fuel_kg;
    for _ in 0..240 {
        sim.step_frame(1.0).unwrap();
    }
    assert!((status(&sim, 13).fuel_kg - (start - 4.0)).abs() < 1e-8);
    assert_eq!(status(&sim, 11).ammunition, 2);
}
#[test]
fn duplicate_delivered_intents_do_not_reserve_cargo_twice() {
    let mut sim = fixture(configuration());
    let intent = AuthorizedIntent {
        intent: PlayerIntent {
            intent_id: Uuid::from_u128(800),
            issuer_role: Uuid::from_u128(200),
            target: Uuid::from_u128(14),
            kind: OrderKind::Logistics {
                command: Command::Cargo {
                    source_id: Uuid::from_u128(5),
                    destination_id: Uuid::from_u128(25),
                    manifest: Stock {
                        fuel_kg: 100.0,
                        ..Default::default()
                    },
                },
            },
            requested_tick: 1,
        },
        authorization: AuthorizationRecord {
            policy_id: Uuid::from_u128(900),
            policy_version: 1,
            requester_role_id: Uuid::from_u128(200),
            granting_role_id: Uuid::from_u128(200),
            request_id: None,
        },
    };
    sim.queue_authorized_intent(intent.clone());
    sim.queue_authorized_intent(intent);
    sim.step();
    let r = sim.world.resource::<Runtime>();
    assert_eq!(r.facilities[&Uuid::from_u128(5)].stock.fuel_kg, 300.0);
    assert_eq!(r.reserved(Uuid::from_u128(5)).fuel_kg, 100.0);
}
#[test]
fn station_is_reserved_through_disconnection_and_breakup_never_undoes_fuel() {
    let mut c = configuration();
    let receiver = c.units[0].clone();
    let mut other = receiver;
    other.unit_id = Uuid::from_u128(12);
    other.profile.id = "fictional-boom-fighter".into();
    other.profile.home = Uuid::from_u128(5);
    c.units[1] = other;
    let mut sim = fixture(c);
    command(
        &mut sim,
        13,
        Command::Support {
            interface: Interface::Boom,
        },
    );
    for id in [11, 12] {
        command(
            &mut sim,
            id,
            Command::Refuel {
                donor_id: Uuid::from_u128(13),
                interface: Interface::Boom,
                amount_kg: 10.0,
            },
        );
    }
    for _ in 0..3 {
        sim.step();
    }
    assert_eq!(status(&sim, 11).transferred_kg, 10.0);
    assert_eq!(status(&sim, 12).transferred_kg, 0.0);
    sim.step();
    assert_eq!(status(&sim, 12).transferred_kg, 0.0);
    assert_eq!(status(&sim, 11).transferred_kg, 10.0);
}
#[test]
fn fully_ready_units_do_not_hold_servicing_slots() {
    let mut c = configuration();
    for u in c
        .units
        .iter_mut()
        .filter(|u| [11, 13].contains(&u.unit_id.as_u128()))
    {
        u.initial_facility = Some(Uuid::from_u128(5));
        u.fuel_kg = 1000.0;
        u.ammunition = 2;
    }
    let mut sim = fixture(c);
    sim.step();
    sim.step();
    assert_eq!(status(&sim, 11).phase, Phase::Ready);
    assert_eq!(status(&sim, 13).phase, Phase::Ready);
}

#[test]
fn supply_ship_delivers_aviation_fuel_and_stores_into_carrier_without_berthing() {
    let mut sim = fixture(configuration());
    command(
        &mut sim,
        16,
        Command::Cargo {
            source_id: Uuid::from_u128(26),
            destination_id: Uuid::from_u128(27),
            manifest: Stock {
                fuel_kg: 100.0,
                ammunition: 2,
                repair_points: 3.0,
            },
        },
    );
    for _ in 0..25 {
        sim.step();
    }
    let r = sim.world.resource::<Runtime>();
    let carrier = &r.facilities[&Uuid::from_u128(27)];
    assert_eq!(carrier.stock.fuel_kg, 140.0);
    assert_eq!(carrier.stock.ammunition, 22);
    assert_eq!(carrier.stock.repair_points, 103.0);
    assert_eq!(r.units[&Uuid::from_u128(16)].status.phase, Phase::Deployed);
    assert_eq!(r.units[&Uuid::from_u128(16)].status.cargo.mass(), 0.0);
    assert_eq!(r.facilities[&Uuid::from_u128(16)].stock.fuel_kg, 500.0);
}
#[test]
fn blocked_storage_cannot_bank_pump_work_and_later_exceed_rate() {
    let mut from = Stock {
        fuel_kg: 100.0,
        ..Default::default()
    };
    let mut to = Stock {
        fuel_kg: 1000.0,
        ..Default::default()
    };
    let cap = to.clone();
    let mut carry = 0.0;
    for _ in 0..1000 {
        carry = transfer_stock(&mut from, &mut to, Some(&cap), 20.0 + carry);
    }
    assert_eq!(carry, 0.0);
    to.fuel_kg = 0.0;
    transfer_stock(&mut from, &mut to, Some(&cap), 20.0 + carry);
    assert_eq!(to.fuel_kg, 20.0);
}
#[test]
fn fractional_cargo_budgets_eventually_transfer_indivisible_stores_without_extra_work() {
    let mut from = Stock {
        ammunition: 2,
        ..Default::default()
    };
    let mut to = Stock::default();
    let mut carry = 0.0;
    for _ in 0..120 {
        carry = transfer_stock(&mut from, &mut to, None, 1.0 / 60.0 + carry);
    }
    assert_eq!(from.ammunition, 0);
    assert_eq!(to.ammunition, 2);
}
#[test]
fn carrier_wind_and_sea_state_restrict_launch_and_recovery() {
    let c = configuration();
    let mut f = c
        .facilities
        .iter()
        .find(|f| f.kind == FacilityKind::Carrier)
        .unwrap()
        .clone();
    f.min_wind_over_deck_mps = 5.0;
    f.max_crosswind_mps = 10.0;
    let p = Position {
        pose: GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 0.0,
        },
        velocity: Velocity::default(),
        side: Side::Blue,
        domain: Domain::Sea,
        destroyed: false,
    };
    let mut w = sensing::Weather {
        wind_north_mps: -10.0,
        ..Default::default()
    };
    assert!(facility_open(&f, &p, &w));
    w.wind_north_mps = 10.0;
    assert!(!facility_open(&f, &p, &w));
    w.wind_north_mps = -10.0;
    w.sea_state = 6.0;
    assert!(!facility_open(&f, &p, &w));
}
#[test]
fn an_immobilized_ship_can_receive_underway_fuel_without_moving_on_empty_tanks() {
    let mut c = configuration();
    c.units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(15))
        .unwrap()
        .fuel_kg = 0.5;
    let mut sim = fixture(c);
    sim.step();
    assert_eq!(status(&sim, 15).phase, Phase::Immobilized);
    command(
        &mut sim,
        16,
        Command::Support {
            interface: Interface::Underway,
        },
    );
    command(
        &mut sim,
        15,
        Command::Refuel {
            donor_id: Uuid::from_u128(16),
            interface: Interface::Underway,
            amount_kg: 100.0,
        },
    );
    for _ in 0..16 {
        sim.step();
    }
    assert!(status(&sim, 15).fuel_kg > 50.0);
    assert_eq!(status(&sim, 15).phase, Phase::Deployed);
}
#[test]
fn automatic_repairs_consume_stock_and_never_revive_destroyed_units() {
    let mut c = configuration();
    let u = c
        .units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(11))
        .unwrap();
    u.initial_facility = Some(Uuid::from_u128(5));
    u.fuel_kg = 1000.0;
    u.ammunition = 2;
    let mut sim = fixture(c);
    sim.configure_combat(crate::combat::CombatConfig {
        units: vec![crate::combat::CombatUnitDefinition {
            unit_id: Uuid::from_u128(11),
            hit_points: 100,
            weapon: None,
        }],
        mission: None,
    })
    .unwrap();
    sim.world
        .resource_mut::<crate::combat::CombatState>()
        .initialize_logistics_health();
    let mut saved = serde_json::to_value(sim.snapshot()).unwrap();
    saved["combat"]["health"][Uuid::from_u128(11).to_string()] = serde_json::json!(50);
    let mut sim = Simulation::restore(serde_json::from_value(saved).unwrap()).unwrap();
    for _ in 0..10 {
        sim.step();
    }
    assert_eq!(
        sim.world
            .resource::<crate::combat::CombatState>()
            .hit_points(Uuid::from_u128(11)),
        Some(70)
    );
    assert_eq!(
        sim.world.resource::<Runtime>().facilities[&Uuid::from_u128(5)]
            .stock
            .repair_points,
        80.0
    );
    let mut saved = serde_json::to_value(sim.snapshot()).unwrap();
    saved["combat"]["health"][Uuid::from_u128(11).to_string()] = serde_json::json!(0);
    let mut sim = Simulation::restore(serde_json::from_value(saved).unwrap()).unwrap();
    for _ in 0..10 {
        sim.step();
    }
    assert_eq!(status(&sim, 11).phase, Phase::Destroyed);
    assert_eq!(
        sim.world
            .resource::<crate::combat::CombatState>()
            .hit_points(Uuid::from_u128(11)),
        Some(0)
    );
    assert_eq!(
        sim.world.resource::<Runtime>().facilities[&Uuid::from_u128(5)]
            .stock
            .repair_points,
        80.0
    );
}
#[test]
fn mission_return_recovers_at_a_facility_and_completes_before_turnaround() {
    use crate::operations::*;
    let mut sim = fixture(configuration());
    let id = Uuid::from_u128(11);
    let task = MissionTask {
        id: Uuid::from_u128(700),
        name: "Recover and turn around".into(),
        kind: MissionKind::Return,
        unit_id: id,
        issuer_role_id: Uuid::from_u128(200),
        objective_id: Uuid::from_u128(800),
        start_tick: 0,
        end_tick: 100,
        route: vec![GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 0.0,
        }],
        speed_mps: 100.0,
        home: GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 8000.0,
        },
        home_facility_id: Some(Uuid::from_u128(5)),
        depends_on: vec![],
        lost_comms: LostCommsProcedure::Continue,
        engagement: EngagementConstraints {
            weapons_release: false,
            minimum_identification: 1.0,
            max_track_age_ticks: 1,
            authorized_objective: None,
        },
        clearance_airspace_ids: vec![],
    };
    sim.receive_tasking(
        id,
        &AirspaceControlOrder {
            plan_id: Uuid::from_u128(999),
            revision: 1,
            volumes: vec![],
        },
        &AirTaskingOrder {
            plan_id: Uuid::from_u128(999),
            revision: 1,
            missions: vec![task],
        },
    );
    for _ in 0..5 {
        sim.step();
    }
    let report = sim.mission_reports(id);
    assert_eq!(report[0].state, MissionState::Completed);
    assert_eq!(status(&sim, 11).facility_id, Some(Uuid::from_u128(5)));
    assert!(status(&sim, 11).fuel_kg < 1000.0);
}
#[test]
fn real_profiles_cannot_use_authored_fictional_tables_as_a_fallback() {
    let mut config = configuration();
    config.units[0].profile.fictional = false;
    let units = config
        .units
        .iter()
        .map(|u| {
            (
                u.unit_id,
                (
                    Side::Blue,
                    if u.unit_id.as_u128() < 15 {
                        Domain::Air
                    } else {
                        Domain::Sea
                    },
                ),
            )
        })
        .chain(
            config
                .facilities
                .iter()
                .map(|f| (f.unit_id, (Side::Blue, Domain::Sea))),
        )
        .collect();
    assert!(config
        .validate(&units)
        .unwrap_err()
        .contains("real logistics parameters require"));
}

#[test]
fn direct_and_mission_fire_share_one_ammunition_ledger() {
    use crate::operations::*;
    let mut sim = fixture(configuration());
    let unit = Uuid::from_u128(11);
    let contact = Uuid::from_u128(9001);
    let shot = Uuid::from_u128(9002);
    sim.configure_combat(crate::combat::CombatConfig {
        units: vec![crate::combat::CombatUnitDefinition {
            unit_id: unit,
            hit_points: 100,
            weapon: Some(crate::combat::TrainingWeapon {
                ammunition: 1,
                range_m: 10000.0,
                speed_mps: 500.0,
                blast_radius_m: 20.0,
                damage: 10,
                max_track_age_ticks: 15,
            }),
        }],
        mission: None,
    })
    .unwrap();
    let track = Track {
        evidence: None,
        uncertainty_m: 100.0,
        assessed_destroyed: None,
        observed_domain: Some(Domain::Air),
        track_id: contact,
        target_side: Some(Side::Red),
        position: GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 0.0,
        },
        identity_confidence: 1.0,
        observed_tick: 0,
        received_tick: 0,
        observed_sidc: String::new(),
    };
    sim.world
        .resource_mut::<KnowledgeBases>()
        .0
        .insert(unit, vec![track.clone()]);
    sim.world
        .resource_mut::<ReportKnowledge>()
        .targets
        .insert((unit, Uuid::from_u128(12)), contact);
    sim.world
        .resource_mut::<crate::combat::CombatState>()
        .designate(shot, unit, Side::Blue, track.clone(), 0)
        .unwrap();
    let task = MissionTask {
        id: Uuid::from_u128(700),
        name: "Defend with the same magazine".into(),
        kind: MissionKind::Defend,
        unit_id: unit,
        issuer_role_id: Uuid::from_u128(200),
        objective_id: Uuid::from_u128(800),
        start_tick: 0,
        end_tick: 100,
        route: vec![track.position],
        speed_mps: 100.0,
        home: track.position,
        home_facility_id: Some(Uuid::from_u128(5)),
        depends_on: vec![],
        lost_comms: LostCommsProcedure::Continue,
        engagement: EngagementConstraints {
            weapons_release: true,
            minimum_identification: 0.8,
            max_track_age_ticks: 15,
            authorized_objective: None,
        },
        clearance_airspace_ids: vec![],
    };
    sim.receive_tasking(
        unit,
        &AirspaceControlOrder {
            plan_id: Uuid::from_u128(999),
            revision: 1,
            volumes: vec![],
        },
        &AirTaskingOrder {
            plan_id: Uuid::from_u128(999),
            revision: 1,
            missions: vec![task],
        },
    );
    sim.queue_authorized_intent(AuthorizedIntent {
        intent: PlayerIntent {
            intent_id: shot,
            issuer_role: Uuid::from_u128(200),
            target: unit,
            kind: OrderKind::Engage { track_id: contact },
            requested_tick: 1,
        },
        authorization: AuthorizationRecord {
            policy_id: Uuid::from_u128(900),
            policy_version: 1,
            requester_role_id: Uuid::from_u128(200),
            granting_role_id: Uuid::from_u128(200),
            request_id: None,
        },
    });
    sim.step();
    assert_eq!(status(&sim, 11).ammunition, 0);
    assert_eq!(
        sim.world
            .resource::<crate::combat::CombatState>()
            .projection(unit, Side::Blue)
            .unwrap()
            .local_shots_in_flight,
        1
    );
    assert!(sim
        .world
        .resource::<Operations>()
        .truth_weapons()
        .is_empty());
}
#[test]
fn transfer_envelopes_apply_to_receiver_and_surface_speed_does_not_subtract_wind() {
    let c = configuration();
    let mut e = c.units[0].profile.envelope.clone();
    e.max_altitude_m = 1000.0;
    let donor = Position {
        pose: GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 900.0,
        },
        velocity: Velocity::default(),
        side: Side::Blue,
        domain: Domain::Air,
        destroyed: false,
    };
    let mut receiver = donor.clone();
    receiver.pose.altitude_m = 1100.0;
    assert!(!transfer_open(
        &receiver,
        &donor,
        &e,
        &sensing::Weather::default()
    ));
    receiver.pose.altitude_m = 900.0;
    receiver.velocity.north_mps = 375.0;
    let mut donor = donor;
    donor.velocity.north_mps = 350.0;
    assert!(!transfer_open(
        &receiver,
        &donor,
        &e,
        &sensing::Weather::default()
    ));
    donor.domain = Domain::Sea;
    donor.velocity = Velocity::default();
    assert_eq!(
        operating_speed(
            &donor,
            &sensing::Weather {
                wind_north_mps: 20.0,
                ..Default::default()
            }
        ),
        0.0
    );
}
#[test]
fn simultaneous_receivers_share_the_donor_pump_budget() {
    let mut c = configuration();
    let mut receiver = c.units[0].clone();
    receiver.unit_id = Uuid::from_u128(12);
    c.units[1] = receiver;
    let donor = c
        .units
        .iter_mut()
        .find(|u| u.unit_id == Uuid::from_u128(13))
        .unwrap();
    donor.profile.outlets = vec![donor.profile.outlets[0].clone(); 2];
    donor.profile.shared_rate_kg_s = 15.0;
    let mut sim = fixture(c);
    command(
        &mut sim,
        13,
        Command::Support {
            interface: Interface::Boom,
        },
    );
    for id in [11, 12] {
        command(
            &mut sim,
            id,
            Command::Refuel {
                donor_id: Uuid::from_u128(13),
                interface: Interface::Boom,
                amount_kg: 100.0,
            },
        );
    }
    for _ in 0..4 {
        sim.step();
    }
    assert_eq!(status(&sim, 11).transferred_kg, 20.0);
    assert_eq!(status(&sim, 12).transferred_kg, 10.0);
    assert_eq!(
        status(&sim, 11).rate_kg_s + status(&sim, 12).rate_kg_s,
        15.0
    );
}
