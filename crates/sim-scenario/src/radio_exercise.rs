//! Small, authored training scenarios with explicit radio limits.
use super::*;

const EXERCISE_DATA: &str = include_str!("../../../data/scenarios/command-link-exercise.v1.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RadioScenarioDefinition {
    id: String,
    title: String,
    description: String,
    version: u32,
    provenance: String,
    units: Vec<ScenarioUnit>,
    radio: TrainingRadio,
    authority: AuthorityDefinition,
    #[serde(default)]
    sensor_report_routes: Vec<SensorReportRoute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainingRadio {
    bit_rate_bps: u64,
    propagation_delay_ns: u64,
    mtu_bytes: usize,
    overhead_bytes: usize,
    max_queue_packets: usize,
    max_queue_bytes: usize,
    shared_medium: String,
    seed: u64,
}

fn parse_radio_scenario(data: &str) -> Result<Scenario, ScenarioError> {
    let definition: RadioScenarioDefinition = serde_json::from_str(data)
        .map_err(|error| ScenarioError::InvalidAuthoredData(error.to_string()))?;
    if definition.id.trim().is_empty()
        || definition.title.trim().is_empty()
        || definition.version == 0
        || definition.provenance.trim().is_empty()
        || definition.radio.shared_medium.trim().is_empty()
        || definition.radio.bit_rate_bps == 0
        || definition.radio.mtu_bytes == 0
        || definition.radio.max_queue_packets == 0
        || definition.radio.max_queue_bytes == 0
    {
        return Err(ScenarioError::InvalidAuthoredData(
            "training scenario metadata and radio limits must be populated".into(),
        ));
    }
    let mut units = definition.units;
    let (mut network, communication_links, mut simulator_options) =
        global_communications(&mut units);
    for channel in &mut network.channels {
        channel.bit_rate_bps = definition.radio.bit_rate_bps;
        channel.propagation_delay_ns = definition.radio.propagation_delay_ns;
        channel.radio = Some(RadioChannel {
            band: FrequencyBand::new(960_000_000, 1_215_000_000),
            interference_response: InterferenceResponse::default(),
        });
    }
    for options in simulator_options.channels.values_mut() {
        options.mtu_bytes = Some(definition.radio.mtu_bytes);
        options.wire_overhead_bytes = definition.radio.overhead_bytes;
        options.shared_medium = Some(definition.radio.shared_medium.clone());
        options.queue = QueueConfig {
            max_packets: Some(definition.radio.max_queue_packets),
            max_bytes: Some(definition.radio.max_queue_bytes),
            discipline: QueueDiscipline::Fifo,
        };
    }
    simulator_options.seed = definition.radio.seed;
    let scenario = Scenario {
        id: definition.id,
        title: definition.title,
        description: definition.description,
        version: definition.version,
        requires_space_catalog: false,
        units,
        network,
        simulator_options,
        communication_links,
        jamming_regions: Vec::new(),
        authority: definition.authority,
        sensor_report_routes: definition.sensor_report_routes,
    };
    scenario.validate()?;
    Ok(scenario)
}

/// A catalog-free exercise whose radio values deliberately expose congestion.
pub fn command_link_exercise_scenario() -> Scenario {
    parse_radio_scenario(EXERCISE_DATA).expect("committed command exercise data must be valid")
}

/// A local sensor report must traverse a slow training radio before reaching the commander.
pub fn sensor_relay_exercise_scenario() -> Scenario {
    parse_radio_scenario(include_str!(
        "../../../data/scenarios/sensor-relay-exercise.v1.json"
    ))
    .expect("committed sensor relay exercise data must be valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advance_for(simulation: &mut Simulation, ticks: u64) -> Vec<sim_core::NetworkEvent> {
        let mut events = simulation.advance_network().unwrap();
        for _ in 0..ticks {
            simulation.step();
            events.extend(simulation.advance_network().unwrap());
        }
        events
    }

    #[test]
    fn command_exercise_has_a_stationary_post_two_aircraft_and_scoped_roles() {
        let scenario = command_link_exercise_scenario();
        assert!(!scenario.requires_space_catalog);
        assert_eq!(scenario.units.len(), 3);
        assert_eq!(scenario.network.devices.len(), 12);
        assert_eq!(scenario.communication_links.len(), 6);
        assert!(scenario
            .units
            .iter()
            .all(|unit| unit.velocity == Velocity::default() && unit.flight_path.is_none()));
        assert_eq!(
            scenario.authority.controlled_units(Uuid::from_u128(20102)),
            vec![Uuid::from_u128(11), Uuid::from_u128(12)]
        );
        assert_eq!(
            scenario.authority.controlled_units(Uuid::from_u128(20103)),
            vec![Uuid::from_u128(11)]
        );
        assert_eq!(
            scenario
                .authority
                .roles
                .iter()
                .filter(|role| role.claimable)
                .count(),
            3
        );
    }

    #[test]
    fn shared_training_radio_delivers_commands_in_serial_order() {
        let scenario = command_link_exercise_scenario();
        let mut simulation = scenario.spawn().unwrap();
        let first = simulation
            .queue_transmission(Uuid::from_u128(5), Uuid::from_u128(11), vec![1; 300])
            .unwrap();
        let second = simulation
            .queue_transmission(Uuid::from_u128(5), Uuid::from_u128(12), vec![2; 300])
            .unwrap();
        let events = advance_for(&mut simulation, 3);
        let deliveries: Vec<_> = events
            .into_iter()
            .filter_map(|event| match event {
                sim_core::NetworkEvent::PacketDelivered { packet, at, .. } => {
                    Some((packet.id(), at.as_nanos()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            deliveries,
            vec![(first, 1_085_000_000), (second, 2_165_000_000)]
        );
    }

    #[test]
    fn exercise_radio_bounds_waiting_commands_without_silent_unlimited_backlog() {
        let scenario = command_link_exercise_scenario();
        let mut simulation = scenario.spawn().unwrap();
        let mut accepted = 0;
        for _ in 0..10 {
            if simulation
                .queue_transmission(Uuid::from_u128(5), Uuid::from_u128(11), vec![1; 300])
                .is_ok()
            {
                accepted += 1;
            }
        }
        let projection = simulation.projection_for(Uuid::from_u128(5), Side::Blue);
        let queued: usize = projection
            .communication_links
            .iter()
            .map(|link| link.queued_packets)
            .sum();
        assert!(queued <= 4);
        // Admission produces packet drop events; accepted injection IDs include
        // those failures so consumers can account for every attempted command.
        assert_eq!(accepted, 10);
        let events = advance_for(&mut simulation, 20);
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, sim_core::NetworkEvent::PacketDropped { .. }))
                .count(),
            5
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, sim_core::NetworkEvent::PacketDelivered { .. }))
                .count(),
            5
        );
    }

    #[test]
    fn invalid_training_limits_and_misspelled_fields_are_rejected() {
        let mut value: serde_json::Value = serde_json::from_str(EXERCISE_DATA).unwrap();
        value["radio"]["bit_rate_bps"] = serde_json::json!(0);
        assert!(matches!(
            parse_radio_scenario(&value.to_string()),
            Err(ScenarioError::InvalidAuthoredData(_))
        ));
        value["radio"]["bit_rate_bps"] = serde_json::json!(2400);
        value["radio"]["packet_limt"] = serde_json::json!(4);
        assert!(matches!(
            parse_radio_scenario(&value.to_string()),
            Err(ScenarioError::InvalidAuthoredData(_))
        ));
    }
}

#[cfg(test)]
mod report_route_tests {
    use super::*;

    #[test]
    fn sensor_exercise_authors_a_local_sensor_and_one_explicit_report_recipient() {
        let scenario = sensor_relay_exercise_scenario();
        assert!(!scenario.requires_space_catalog);
        assert_eq!(scenario.units.len(), 4);
        assert_eq!(scenario.sensor_report_routes.len(), 1);
        assert_eq!(
            scenario.sensor_report_routes[0].origin_role_id,
            Uuid::from_u128(20103)
        );
        assert_eq!(
            scenario.sensor_report_routes[0].recipient_unit_id,
            Uuid::from_u128(5)
        );
        assert_eq!(scenario.sensor_report_routes[0].interval_ticks, 10);
        assert_eq!(
            scenario
                .units
                .iter()
                .filter(|unit| unit.sensor.is_some())
                .count(),
            1
        );
        let mut simulation = scenario.spawn().unwrap();
        simulation.step();
        assert_eq!(
            simulation
                .projection_for(Uuid::from_u128(11), Side::Blue)
                .tracks
                .len(),
            1
        );
        assert!(simulation
            .projection_for(Uuid::from_u128(5), Side::Blue)
            .tracks
            .is_empty());
        assert!(scenario
            .units
            .iter()
            .all(|unit| !unit.network_device_ids.is_empty()));
    }

    #[test]
    fn report_routes_reject_unknown_roles_zero_intervals_friendly_sensor_absence_and_enemy_recipients(
    ) {
        for (origin, recipient, interval) in [
            (999, 5, 10),
            (20103, 5, 0),
            (20104, 5, 10),
            (20103, 51, 10),
            (20103, 11, 10),
        ] {
            let mut scenario = sensor_relay_exercise_scenario();
            scenario.sensor_report_routes[0] = SensorReportRoute {
                origin_role_id: Uuid::from_u128(origin),
                recipient_unit_id: Uuid::from_u128(recipient),
                interval_ticks: interval,
            };
            assert!(scenario
                .validate()
                .unwrap_err()
                .to_string()
                .contains("sensor report routes"));
        }
        let mut scenario = sensor_relay_exercise_scenario();
        scenario
            .sensor_report_routes
            .push(scenario.sensor_report_routes[0].clone());
        assert!(scenario
            .validate()
            .unwrap_err()
            .to_string()
            .contains("sensor report routes"));
        let mut scenario = sensor_relay_exercise_scenario();
        scenario.communication_links.retain(|link| {
            !(link.from_entity_id == Uuid::from_u128(11) && link.to_entity_id == Uuid::from_u128(5))
        });
        assert!(scenario
            .validate()
            .unwrap_err()
            .to_string()
            .contains("sensor report routes"));
    }
}
