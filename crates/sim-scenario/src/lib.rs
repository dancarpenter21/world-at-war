//! Versioned scenario definitions and spawning.

use c3mesh::{
    ChannelConfig, ChannelId, ChannelState, DeviceConfig, DeviceId, DeviceKind, FrequencyBand,
    InterferenceResponse, NetworkConfig, RadioChannel,
};
use serde::{Deserialize, Serialize};
use sim_core::{
    AuthorityDecisionStep, AuthorityDefinition, AuthorityPolicy, AuthorityRelationship,
    AuthorityRelationshipKind, AuthorityRoleDefinition, AuthorityRoleKind,
    CommunicationLinkDefinition, CommunicationsConfig, CyclicFlightPath, Domain, FlightWaypoint,
    GeoPose, JammingRegion, PlatformSpawn, Sensor, Side, Simulation, Velocity, ACTION_ENGAGE,
    ACTION_MOVE, ACTION_SPACE_SUPPORT, ACTION_STRATEGIC_SATELLITE,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    pub description: String,
    pub version: u32,
    pub requires_space_catalog: bool,
    pub units: Vec<ScenarioUnit>,
    pub network: NetworkConfig,
    pub communication_links: Vec<CommunicationLinkDefinition>,
    pub jamming_regions: Vec<JammingRegion>,
    pub authority: AuthorityDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioUnit {
    pub id: Uuid,
    pub name: String,
    pub side: Side,
    pub domain: Domain,
    pub sidc: String,
    pub position: GeoPose,
    pub velocity: Velocity,
    pub sensor: Option<Sensor>,
    pub network_device_ids: Vec<DeviceId>,
    pub flight_path: Option<CyclicFlightPath>,
}

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("scenario must include at least one unit")]
    MissingUnits,
    #[error("scenario includes duplicate unit id {0}")]
    DuplicateUnit(Uuid),
    #[error("entity {0} has an invalid MIL-STD-2525D SIDC")]
    InvalidSidc(Uuid),
    #[error("invalid authority definition: {0}")]
    InvalidAuthority(String),
    #[error("invalid simulation definition: {0}")]
    InvalidSimulation(String),
}

impl Scenario {
    pub fn validate(&self) -> Result<(), ScenarioError> {
        let mut ids = std::collections::BTreeSet::new();
        if self.units.is_empty() {
            return Err(ScenarioError::MissingUnits);
        }
        for unit in &self.units {
            if !ids.insert(unit.id) {
                return Err(ScenarioError::DuplicateUnit(unit.id));
            }
            if unit.sidc.len() != 30
                || !unit
                    .sidc
                    .chars()
                    .all(|character| character.is_ascii_digit())
            {
                return Err(ScenarioError::InvalidSidc(unit.id));
            }
        }
        self.authority
            .validate(&ids)
            .map_err(ScenarioError::InvalidAuthority)?;
        let platforms = self.platforms();
        Simulation::validate_configuration(&platforms, &self.communications())
            .map_err(|error| ScenarioError::InvalidSimulation(error.to_string()))?;
        Ok(())
    }

    pub fn spawn(&self) -> Result<Simulation, ScenarioError> {
        self.validate()?;
        Simulation::new(self.platforms(), self.communications())
            .map_err(|error| ScenarioError::InvalidSimulation(error.to_string()))
    }

    fn platforms(&self) -> Vec<PlatformSpawn> {
        self.units
            .iter()
            .map(|unit| PlatformSpawn {
                id: unit.id,
                name: unit.name.clone(),
                side: unit.side,
                domain: unit.domain,
                pose: unit.position,
                velocity: unit.velocity,
                sensor: unit.sensor,
                network_device_ids: unit.network_device_ids.clone(),
                flight_path: unit.flight_path.clone(),
                sidc: unit.sidc.clone(),
            })
            .collect()
    }

    fn communications(&self) -> CommunicationsConfig {
        CommunicationsConfig {
            network: self.network.clone(),
            links: self.communication_links.clone(),
            jamming_regions: self.jamming_regions.clone(),
        }
    }
}

pub fn global_crisis_scenario() -> Scenario {
    let blue = [
        (
            "White House Situation Room",
            Domain::Land,
            38.8977,
            -77.0365,
        ),
        ("Pentagon NMCC", Domain::Land, 38.8719, -77.0563),
        ("Joint Base Andrews", Domain::Land, 38.8108, -76.8670),
        ("Naval Station Norfolk", Domain::Sea, 36.9460, -76.3290),
        ("Andersen Air Force Base", Domain::Land, 13.5840, 144.9300),
        ("Joint Base Pearl Harbor", Domain::Sea, 21.3490, -157.9440),
        ("Ramstein Air Base", Domain::Land, 49.4369, 7.6003),
        ("Yokota Air Base", Domain::Land, 35.7485, 139.3480),
        (
            "Diego Garcia Support Facility",
            Domain::Sea,
            -7.3195,
            72.4229,
        ),
        (
            "Cheyenne Mountain Station",
            Domain::Land,
            38.7445,
            -104.8460,
        ),
        ("F-35A CAP Alpha 1", Domain::Air, 38.1, -76.7),
        ("F-35A CAP Alpha 2", Domain::Air, 38.2, -76.6),
        ("F-35C Pacific 1", Domain::Air, 22.0, 145.0),
        ("F-35C Pacific 2", Domain::Air, 22.2, 145.2),
        ("F-15EX Europe 1", Domain::Air, 50.0, 8.0),
        ("F-15EX Europe 2", Domain::Air, 50.1, 8.2),
        ("B-2A Spirit 1", Domain::Air, 39.0, -95.0),
        ("B-52H Global 1", Domain::Air, 34.0, -105.0),
        ("E-3G Sentry Atlantic", Domain::Air, 42.0, -45.0),
        ("E-2D Hawkeye Pacific", Domain::Air, 20.0, 150.0),
        ("KC-46A Atlantic 1", Domain::Air, 40.0, -55.0),
        ("KC-46A Pacific 1", Domain::Air, 25.0, 155.0),
        ("P-8A Poseidon Atlantic", Domain::Air, 35.0, -40.0),
        ("P-8A Poseidon Pacific", Domain::Air, 18.0, 148.0),
        ("MQ-9A Europe", Domain::Air, 47.0, 15.0),
        ("RQ-4 Global Hawk", Domain::Air, 30.0, 135.0),
        ("USS Gerald R. Ford", Domain::Sea, 37.0, -50.0),
        ("USS Arleigh Burke", Domain::Sea, 36.5, -49.0),
        ("USS Thomas Hudner", Domain::Sea, 37.5, -51.0),
        ("USS Ronald Reagan", Domain::Sea, 20.0, 145.0),
        ("USS John Finn", Domain::Sea, 19.5, 144.5),
        ("USS Rafael Peralta", Domain::Sea, 20.5, 145.5),
        ("USS Virginia", Domain::Undersea, 38.0, -42.0),
        ("USS Hawaii", Domain::Undersea, 18.0, 150.0),
        ("USNS John Lewis", Domain::Sea, 21.0, 146.0),
        ("USNS Supply", Domain::Sea, 36.0, -52.0),
        ("1st Armored Brigade", Domain::Land, 52.0, 20.0),
        ("2nd Stryker Brigade", Domain::Land, 51.0, 22.0),
        ("Marine Littoral Regiment", Domain::Land, 14.0, 145.0),
        ("HIMARS Battalion Pacific", Domain::Land, 13.6, 144.9),
        ("Patriot Battalion Europe", Domain::Land, 50.0, 9.0),
        ("THAAD Battery Guam", Domain::Land, 13.5, 144.8),
        ("JTAC Team Saber", Domain::Land, 51.5, 21.0),
        ("Joint Logistics Group", Domain::Land, 49.5, 8.0),
        ("Cyber Mission Force", Domain::Cyber, 39.0, -77.0),
        ("Defensive Network Cell", Domain::Cyber, 38.9, -77.05),
        ("SATCOM Control Element", Domain::Space, 38.7, -104.8),
        ("Space Domain Awareness Cell", Domain::Space, 38.8, -104.9),
    ];
    let red = [
        ("Red National Command", Domain::Land, 55.75, 37.62),
        ("Red Air Operations Center", Domain::Land, 54.5, 38.0),
        ("Red Fighter Flight 1", Domain::Air, 54.0, 35.0),
        ("Red Fighter Flight 2", Domain::Air, 53.5, 36.0),
        ("Red Bomber Flight", Domain::Air, 60.0, 45.0),
        ("Red AEW Aircraft", Domain::Air, 56.0, 40.0),
        ("Red Tanker", Domain::Air, 57.0, 42.0),
        ("Red Surface Group Flag", Domain::Sea, 64.0, 5.0),
        ("Red Surface Escort 1", Domain::Sea, 63.5, 4.5),
        ("Red Surface Escort 2", Domain::Sea, 64.5, 5.5),
        ("Red Attack Submarine", Domain::Undersea, 60.0, -5.0),
        ("Red Armor Brigade", Domain::Land, 53.0, 28.0),
        ("Red Missile Brigade", Domain::Land, 54.0, 30.0),
        ("Red Air Defense Regiment", Domain::Land, 55.0, 32.0),
        ("Red EW Battalion", Domain::Cyber, 55.5, 34.0),
        ("Red Cyber Unit", Domain::Cyber, 55.7, 37.0),
    ];
    let mut units = Vec::with_capacity(64);
    for (index, (name, domain, latitude, longitude)) in blue.into_iter().enumerate() {
        units.push(unit(
            index as u128 + 1,
            name,
            Side::Blue,
            domain,
            latitude,
            longitude,
        ));
    }
    for (index, (name, domain, latitude, longitude)) in red.into_iter().enumerate() {
        units.push(unit(
            index as u128 + 49,
            name,
            Side::Red,
            domain,
            latitude,
            longitude,
        ));
    }
    let authority = global_crisis_authority();
    let network = NetworkConfig {
        devices: units
            .iter()
            .flat_map(|unit| {
                unit.network_device_ids
                    .iter()
                    .cloned()
                    .map(|id| DeviceConfig {
                        id,
                        kind: DeviceKind::Sink,
                        mobility: Default::default(),
                        interference: vec![],
                    })
            })
            .collect(),
        channels: vec![],
    };
    Scenario {
        id: "global-crisis.v2".into(),
        title: "Global Crisis".into(),
        description: "A combined-domain global crisis directed from the White House and Pentagon."
            .into(),
        version: 2,
        requires_space_catalog: true,
        units,
        network,
        communication_links: vec![],
        jamming_regions: vec![],
        authority,
    }
}

fn global_crisis_authority() -> AuthorityDefinition {
    let role_specs = [
        (
            101,
            "President of the United States",
            Side::Blue,
            AuthorityRoleKind::NationalCommand,
            1,
            true,
            false,
        ),
        (
            102,
            "Secretary of Defense",
            Side::Blue,
            AuthorityRoleKind::DefenseSecretary,
            2,
            true,
            false,
        ),
        (
            103,
            "Chairman, Joint Chiefs of Staff",
            Side::Blue,
            AuthorityRoleKind::JointStaff,
            2,
            true,
            false,
        ),
        (
            104,
            "Supported Combatant Commander",
            Side::Blue,
            AuthorityRoleKind::CombatantCommander,
            2,
            true,
            false,
        ),
        (
            105,
            "Joint Force Commander",
            Side::Blue,
            AuthorityRoleKind::JointForceCommander,
            2,
            true,
            false,
        ),
        (
            106,
            "Joint Force Air Component Commander",
            Side::Blue,
            AuthorityRoleKind::ComponentCommander,
            5,
            true,
            false,
        ),
        (
            107,
            "Joint Force Maritime Component Commander",
            Side::Blue,
            AuthorityRoleKind::ComponentCommander,
            6,
            true,
            false,
        ),
        (
            108,
            "Joint Force Land Component Commander",
            Side::Blue,
            AuthorityRoleKind::ComponentCommander,
            37,
            true,
            false,
        ),
        (
            109,
            "Joint Force Cyber Component Commander",
            Side::Blue,
            AuthorityRoleKind::ComponentCommander,
            45,
            true,
            false,
        ),
        (
            110,
            "Space Coordinating Authority",
            Side::Blue,
            AuthorityRoleKind::ComponentCommander,
            48,
            true,
            false,
        ),
        (
            111,
            "Tactical Flight Lead",
            Side::Blue,
            AuthorityRoleKind::TacticalCommander,
            11,
            true,
            false,
        ),
        (
            112,
            "United States Space Command Commander",
            Side::Blue,
            AuthorityRoleKind::CombatantCommander,
            10,
            true,
            false,
        ),
        (
            113,
            "Space Operations Component Commander",
            Side::Blue,
            AuthorityRoleKind::ComponentCommander,
            47,
            true,
            false,
        ),
        (
            201,
            "Red National Command",
            Side::Red,
            AuthorityRoleKind::NationalCommand,
            49,
            false,
            true,
        ),
        (
            202,
            "Red Joint Commander AI",
            Side::Red,
            AuthorityRoleKind::JointForceCommander,
            50,
            false,
            true,
        ),
    ];
    let roles = role_specs
        .into_iter()
        .map(
            |(id, name, side, kind, location, claimable, ai_controlled)| AuthorityRoleDefinition {
                id: Uuid::from_u128(id),
                name: name.into(),
                side,
                kind,
                location_unit_id: Uuid::from_u128(location),
                claimable,
                ai_controlled,
            },
        )
        .collect();
    let mut relationships = Vec::new();
    let mut next_edge = 1_000u128;
    let mut add_role_edge = |superior: u128, subordinate: u128, kind| {
        relationships.push(AuthorityRelationship {
            id: Uuid::from_u128(next_edge),
            superior_role_id: Uuid::from_u128(superior),
            subordinate_role_id: Some(Uuid::from_u128(subordinate)),
            subordinate_unit_id: None,
            kind,
        });
        next_edge += 1;
    };
    add_role_edge(101, 102, AuthorityRelationshipKind::NationalCommand);
    add_role_edge(102, 104, AuthorityRelationshipKind::NationalCommand);
    add_role_edge(104, 105, AuthorityRelationshipKind::Opcon);
    for component in [106, 107, 108, 109] {
        add_role_edge(105, component, AuthorityRelationshipKind::Opcon);
    }
    add_role_edge(106, 111, AuthorityRelationshipKind::Tacon);
    add_role_edge(102, 112, AuthorityRelationshipKind::NationalCommand);
    add_role_edge(112, 113, AuthorityRelationshipKind::Opcon);
    add_role_edge(102, 103, AuthorityRelationshipKind::Advisory);
    add_role_edge(103, 104, AuthorityRelationshipKind::Transmit);
    add_role_edge(105, 110, AuthorityRelationshipKind::Support);
    add_role_edge(110, 112, AuthorityRelationshipKind::Support);
    add_role_edge(201, 202, AuthorityRelationshipKind::Opcon);
    let _ = add_role_edge;

    let mut add_unit_edge = |superior: u128, unit: u128, kind| {
        relationships.push(AuthorityRelationship {
            id: Uuid::from_u128(next_edge),
            superior_role_id: Uuid::from_u128(superior),
            subordinate_role_id: None,
            subordinate_unit_id: Some(Uuid::from_u128(unit)),
            kind,
        });
        next_edge += 1;
    };
    add_unit_edge(101, 1, AuthorityRelationshipKind::NationalCommand);
    add_unit_edge(102, 2, AuthorityRelationshipKind::NationalCommand);
    for unit in [3, 5, 7, 8, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26] {
        add_unit_edge(106, unit, AuthorityRelationshipKind::Tacon);
    }
    for unit in [11, 12, 13, 14] {
        add_unit_edge(111, unit, AuthorityRelationshipKind::Tacon);
    }
    for unit in [4, 6, 9, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36] {
        add_unit_edge(107, unit, AuthorityRelationshipKind::Tacon);
    }
    for unit in 37..=44 {
        add_unit_edge(108, unit, AuthorityRelationshipKind::Tacon);
    }
    for unit in [45, 46] {
        add_unit_edge(109, unit, AuthorityRelationshipKind::Tacon);
    }
    for unit in [10, 47, 48] {
        add_unit_edge(113, unit, AuthorityRelationshipKind::Tacon);
    }
    add_unit_edge(201, 49, AuthorityRelationshipKind::NationalCommand);
    for unit in 50..=64 {
        add_unit_edge(202, unit, AuthorityRelationshipKind::Tacon);
    }
    let _ = add_unit_edge;

    let policy = |id,
                  name: &str,
                  action: &str,
                  targets: Vec<u128>,
                  direct: Vec<u128>,
                  request: Vec<u128>,
                  decisions: Vec<u128>,
                  executable| AuthorityPolicy {
        id: Uuid::from_u128(id),
        name: name.into(),
        action: action.into(),
        target_unit_ids: targets.into_iter().map(Uuid::from_u128).collect(),
        direct_role_ids: direct.into_iter().map(Uuid::from_u128).collect(),
        request_role_ids: request.into_iter().map(Uuid::from_u128).collect(),
        decision_steps: decisions
            .into_iter()
            .map(|role_id| AuthorityDecisionStep {
                role_id: Uuid::from_u128(role_id),
                vacant_delay_ticks: 60,
                approve_probability_bps: 5_000,
            })
            .collect(),
        notify_role_ids: vec![Uuid::from_u128(103)],
        executable,
    };
    let policies = vec![
        policy(
            301,
            "Air movement authority",
            ACTION_MOVE,
            vec![3, 5, 7, 8, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26],
            vec![101, 102, 104, 105, 106],
            vec![],
            vec![],
            true,
        ),
        policy(
            302,
            "Tactical air movement authority",
            ACTION_MOVE,
            vec![11, 12, 13, 14],
            vec![101, 102, 104, 105, 106, 111],
            vec![],
            vec![],
            true,
        ),
        policy(
            303,
            "Maritime movement authority",
            ACTION_MOVE,
            [4, 6, 9, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36].to_vec(),
            vec![101, 102, 104, 105, 107],
            vec![],
            vec![],
            true,
        ),
        policy(
            304,
            "Land movement authority",
            ACTION_MOVE,
            (37..=44).collect(),
            vec![101, 102, 104, 105, 108],
            vec![],
            vec![],
            true,
        ),
        policy(
            305,
            "Cyber task authority",
            ACTION_MOVE,
            vec![45, 46],
            vec![101, 102, 104, 105, 109],
            vec![],
            vec![],
            true,
        ),
        policy(
            306,
            "Space force movement authority",
            ACTION_MOVE,
            vec![10, 47, 48],
            vec![101, 102, 112, 113],
            vec![],
            vec![],
            true,
        ),
        policy(
            307,
            "Joint engagement authority",
            ACTION_ENGAGE,
            (3..=46).filter(|unit| *unit != 10).collect(),
            vec![101, 102, 104, 105],
            vec![106, 107, 108, 109, 111],
            vec![105],
            true,
        ),
        policy(
            308,
            "Space support request",
            ACTION_SPACE_SUPPORT,
            vec![47, 48],
            vec![101, 102, 112, 113],
            vec![105, 106, 107, 108, 109, 110, 111],
            vec![112],
            false,
        ),
        policy(
            309,
            "Strategic satellite maneuver",
            ACTION_STRATEGIC_SATELLITE,
            vec![47, 48],
            vec![101],
            vec![105, 106, 107, 108, 109, 110, 111, 112, 113],
            vec![101],
            false,
        ),
        policy(
            310,
            "Red movement authority",
            ACTION_MOVE,
            (50..=64).collect(),
            vec![201, 202],
            vec![],
            vec![],
            true,
        ),
        policy(
            311,
            "White House movement authority",
            ACTION_MOVE,
            vec![1],
            vec![101],
            vec![],
            vec![],
            true,
        ),
        policy(
            312,
            "Pentagon movement authority",
            ACTION_MOVE,
            vec![2],
            vec![101, 102],
            vec![],
            vec![],
            true,
        ),
        policy(
            313,
            "Red national movement authority",
            ACTION_MOVE,
            vec![49],
            vec![201],
            vec![],
            vec![],
            true,
        ),
    ];
    AuthorityDefinition {
        version: 1,
        roles,
        relationships,
        policies,
    }
}

pub fn jammed_flight_scenario() -> Scenario {
    const CENTER_LATITUDE: f64 = 36.0;
    const CENTER_LONGITUDE: f64 = -115.0;
    const WEST_LONGITUDE: f64 = -115.09;
    const EAST_LONGITUDE: f64 = -114.91;
    const SOUTH_LATITUDE: f64 = 35.92;
    const NORTH_LATITUDE: f64 = 36.12;
    const ALTITUDE_M: f64 = 8_000.0;
    const RADIO_BAND: FrequencyBand = FrequencyBand::new(300_000_000, 320_000_000);

    let aircraft_1 = Uuid::from_u128(10_001);
    let aircraft_2 = Uuid::from_u128(10_002);
    let pilot_1 = Uuid::from_u128(10_101);
    let pilot_2 = Uuid::from_u128(10_102);
    let aircraft_1_tx = DeviceId::new("jam-flight-aircraft-1-tx");
    let aircraft_1_rx = DeviceId::new("jam-flight-aircraft-1-rx");
    let aircraft_2_tx = DeviceId::new("jam-flight-aircraft-2-tx");
    let aircraft_2_rx = DeviceId::new("jam-flight-aircraft-2-rx");
    let aircraft_1_channel = ChannelId::new("jam-flight-aircraft-1-to-2");
    let aircraft_2_channel = ChannelId::new("jam-flight-aircraft-2-to-1");

    let flight_path = |north_latitude: f64, south_latitude: f64| CyclicFlightPath {
        period_ticks: 50,
        waypoints: vec![
            FlightWaypoint {
                at_tick: 0,
                position: GeoPose {
                    latitude_deg: north_latitude,
                    longitude_deg: WEST_LONGITUDE,
                    altitude_m: ALTITUDE_M,
                },
            },
            FlightWaypoint {
                at_tick: 10,
                position: GeoPose {
                    latitude_deg: north_latitude,
                    longitude_deg: CENTER_LONGITUDE,
                    altitude_m: ALTITUDE_M,
                },
            },
            FlightWaypoint {
                at_tick: 20,
                position: GeoPose {
                    latitude_deg: north_latitude,
                    longitude_deg: EAST_LONGITUDE,
                    altitude_m: ALTITUDE_M,
                },
            },
            FlightWaypoint {
                at_tick: 30,
                position: GeoPose {
                    latitude_deg: south_latitude,
                    longitude_deg: EAST_LONGITUDE,
                    altitude_m: ALTITUDE_M,
                },
            },
            FlightWaypoint {
                at_tick: 40,
                position: GeoPose {
                    latitude_deg: south_latitude,
                    longitude_deg: WEST_LONGITUDE,
                    altitude_m: ALTITUDE_M,
                },
            },
        ],
    };
    let units = vec![
        ScenarioUnit {
            id: aircraft_1,
            name: "Blue Flight 1".into(),
            side: Side::Blue,
            domain: Domain::Air,
            sidc: sidc(Side::Blue, Domain::Air).into(),
            position: GeoPose {
                latitude_deg: CENTER_LATITUDE,
                longitude_deg: WEST_LONGITUDE,
                altitude_m: ALTITUDE_M,
            },
            velocity: Velocity {
                north_mps: 0.0,
                east_mps: 0.0,
                climb_mps: 0.0,
            },
            sensor: None,
            network_device_ids: vec![aircraft_1_tx.clone(), aircraft_1_rx.clone()],
            flight_path: Some(flight_path(CENTER_LATITUDE, SOUTH_LATITUDE)),
        },
        ScenarioUnit {
            id: aircraft_2,
            name: "Blue Flight 2".into(),
            side: Side::Blue,
            domain: Domain::Air,
            sidc: sidc(Side::Blue, Domain::Air).into(),
            position: GeoPose {
                latitude_deg: NORTH_LATITUDE,
                longitude_deg: WEST_LONGITUDE,
                altitude_m: ALTITUDE_M,
            },
            velocity: Velocity {
                north_mps: 0.0,
                east_mps: 0.0,
                climb_mps: 0.0,
            },
            sensor: None,
            network_device_ids: vec![aircraft_2_tx.clone(), aircraft_2_rx.clone()],
            flight_path: Some(flight_path(NORTH_LATITUDE, NORTH_LATITUDE + 0.08)),
        },
    ];
    let network = NetworkConfig {
        devices: vec![
            DeviceConfig {
                id: aircraft_1_tx.clone(),
                kind: DeviceKind::Source {
                    egress: aircraft_1_channel.clone(),
                },
                mobility: Default::default(),
                interference: vec![],
            },
            DeviceConfig {
                id: aircraft_1_rx.clone(),
                kind: DeviceKind::Sink,
                mobility: Default::default(),
                interference: vec![],
            },
            DeviceConfig {
                id: aircraft_2_tx.clone(),
                kind: DeviceKind::Source {
                    egress: aircraft_2_channel.clone(),
                },
                mobility: Default::default(),
                interference: vec![],
            },
            DeviceConfig {
                id: aircraft_2_rx.clone(),
                kind: DeviceKind::Sink,
                mobility: Default::default(),
                interference: vec![],
            },
        ],
        channels: vec![
            ChannelConfig {
                id: aircraft_1_channel.clone(),
                endpoints: [aircraft_1_tx.clone(), aircraft_2_rx.clone()],
                bit_rate_bps: 1_000_000,
                propagation_delay_ns: 100_000,
                state: ChannelState::Operational,
                distance: None,
                radio: Some(RadioChannel {
                    band: RADIO_BAND,
                    interference_response: InterferenceResponse::default(),
                }),
            },
            ChannelConfig {
                id: aircraft_2_channel.clone(),
                endpoints: [aircraft_2_tx.clone(), aircraft_1_rx.clone()],
                bit_rate_bps: 1_000_000,
                propagation_delay_ns: 100_000,
                state: ChannelState::Operational,
                distance: None,
                radio: Some(RadioChannel {
                    band: RADIO_BAND,
                    interference_response: InterferenceResponse::default(),
                }),
            },
        ],
    };
    let communication_links = vec![
        CommunicationLinkDefinition {
            id: "aircraft-1-to-aircraft-2".into(),
            from_entity_id: aircraft_1,
            to_entity_id: aircraft_2,
            source_device_id: aircraft_1_tx,
            destination_device_id: aircraft_2_rx,
            channel_id: aircraft_1_channel,
        },
        CommunicationLinkDefinition {
            id: "aircraft-2-to-aircraft-1".into(),
            from_entity_id: aircraft_2,
            to_entity_id: aircraft_1,
            source_device_id: aircraft_2_tx,
            destination_device_id: aircraft_1_rx,
            channel_id: aircraft_2_channel,
        },
    ];
    let authority = AuthorityDefinition {
        version: 1,
        roles: vec![
            AuthorityRoleDefinition {
                id: pilot_1,
                name: "Pilot, Blue Flight 1".into(),
                side: Side::Blue,
                kind: AuthorityRoleKind::Pilot,
                location_unit_id: aircraft_1,
                claimable: true,
                ai_controlled: false,
            },
            AuthorityRoleDefinition {
                id: pilot_2,
                name: "Pilot, Blue Flight 2".into(),
                side: Side::Blue,
                kind: AuthorityRoleKind::Pilot,
                location_unit_id: aircraft_2,
                claimable: true,
                ai_controlled: false,
            },
        ],
        relationships: vec![
            AuthorityRelationship {
                id: Uuid::from_u128(10_201),
                superior_role_id: pilot_1,
                subordinate_role_id: None,
                subordinate_unit_id: Some(aircraft_1),
                kind: AuthorityRelationshipKind::Tacon,
            },
            AuthorityRelationship {
                id: Uuid::from_u128(10_202),
                superior_role_id: pilot_2,
                subordinate_role_id: None,
                subordinate_unit_id: Some(aircraft_2),
                kind: AuthorityRelationshipKind::Tacon,
            },
        ],
        policies: vec![
            AuthorityPolicy {
                id: Uuid::from_u128(10_301),
                name: "Blue Flight 1 movement authority".into(),
                action: ACTION_MOVE.into(),
                target_unit_ids: vec![aircraft_1],
                direct_role_ids: vec![pilot_1],
                request_role_ids: vec![],
                decision_steps: vec![],
                notify_role_ids: vec![],
                executable: true,
            },
            AuthorityPolicy {
                id: Uuid::from_u128(10_302),
                name: "Blue Flight 2 movement authority".into(),
                action: ACTION_MOVE.into(),
                target_unit_ids: vec![aircraft_2],
                direct_role_ids: vec![pilot_2],
                request_role_ids: vec![],
                decision_steps: vec![],
                notify_role_ids: vec![],
                executable: true,
            },
        ],
    };

    Scenario {
        id: "jammed-flight.v1".into(),
        title: "Jammed Flight Test".into(),
        description:
            "Two Blue aircraft repeatedly cross a receiver-jamming region while testing a directional radio link."
                .into(),
        version: 1,
        requires_space_catalog: false,
        units,
        network,
        communication_links,
        jamming_regions: vec![JammingRegion {
            id: "training-jammer".into(),
            name: "Training Jammer".into(),
            center: GeoPose {
                latitude_deg: CENTER_LATITUDE,
                longitude_deg: CENTER_LONGITUDE,
                altitude_m: 0.0,
            },
            radius_m: 5_000.0,
            band: RADIO_BAND,
            jammed: 1.0,
        }],
        authority,
    }
}

fn unit(
    id: u128,
    name: &str,
    side: Side,
    domain: Domain,
    latitude: f64,
    longitude: f64,
) -> ScenarioUnit {
    let airborne = domain == Domain::Air;
    let entity_id = Uuid::from_u128(id);
    ScenarioUnit {
        id: entity_id,
        name: name.into(),
        side,
        domain,
        sidc: sidc(side, domain).into(),
        position: GeoPose {
            latitude_deg: latitude,
            longitude_deg: longitude,
            altitude_m: if airborne { 8_000.0 } else { 0.0 },
        },
        velocity: Velocity {
            north_mps: 0.0,
            east_mps: if airborne { 180.0 } else { 0.0 },
            climb_mps: 0.0,
        },
        sensor: Some(Sensor {
            range_m: if airborne { 180_000.0 } else { 80_000.0 },
            identification_range_m: 35_000.0,
        }),
        network_device_ids: vec![DeviceId::new(format!("entity-{entity_id}-network"))],
        flight_path: None,
    }
}

fn sidc(side: Side, domain: Domain) -> &'static str {
    match (side, domain) {
        (Side::Blue, Domain::Air) => "100301000011010000000000000000",
        (Side::Red, Domain::Air) => "100601000011010000000000000000",
        (Side::Blue, Domain::Sea) => "100330000012010000000000000000",
        (Side::Red, Domain::Sea) => "100630000012010000000000000000",
        (Side::Blue, Domain::Undersea) => "100335000012010000000000000000",
        (Side::Red, Domain::Undersea) => "100635000012010000000000000000",
        (Side::Blue, Domain::Space) => "100305000011010000000000000000",
        (Side::Red, Domain::Space) => "100605000011010000000000000000",
        (Side::Blue, Domain::Cyber) => "100340000012110000000000000000",
        (Side::Red, Domain::Cyber) => "100640000012110000000000000000",
        (Side::Blue, Domain::Land) => "100310000012110000000000000000",
        (Side::Red, Domain::Land) => "100610000012110000000000000000",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use c3mesh::DropReason;
    use sim_core::CommunicationOutcome;

    #[test]
    fn global_crisis_has_required_entities_and_roles() {
        let scenario = global_crisis_scenario();
        scenario.validate().unwrap();
        assert_eq!(scenario.units.len(), 64);
        assert_eq!(scenario.authority.roles.len(), 15);
        assert!(scenario
            .units
            .iter()
            .any(|unit| unit.name.contains("White House")));
        assert!(scenario
            .units
            .iter()
            .any(|unit| unit.name.contains("Pentagon")));
        assert!(scenario
            .units
            .iter()
            .all(|unit| !unit.network_device_ids.is_empty()));
        assert_eq!(scenario.network.devices.len(), scenario.units.len());
    }

    #[test]
    fn jammed_flight_is_directional_and_recovers() {
        let scenario = jammed_flight_scenario();
        scenario.validate().unwrap();
        assert_eq!(scenario.units.len(), 2);
        assert_eq!(scenario.authority.roles.len(), 2);
        assert!(scenario
            .authority
            .roles
            .iter()
            .all(|role| role.kind == AuthorityRoleKind::Pilot));
        let aircraft_1 = scenario.units[0].id;
        let aircraft_2 = scenario.units[1].id;
        let mut simulation = scenario.spawn().unwrap();

        assert!(matches!(
            simulation
                .transmit(aircraft_1, aircraft_2, b"initial one to two".to_vec())
                .unwrap(),
            CommunicationOutcome::Delivered { .. }
        ));
        assert!(matches!(
            simulation
                .transmit(aircraft_2, aircraft_1, b"initial two to one".to_vec())
                .unwrap(),
            CommunicationOutcome::Delivered { .. }
        ));

        for _ in 0..10 {
            simulation.step();
        }
        let projection = simulation.projection_for(aircraft_1, Side::Blue);
        assert!(
            projection
                .own_units
                .iter()
                .find(|unit| unit.id == aircraft_1)
                .unwrap()
                .receiver_jammed
        );
        assert!(
            projection
                .communication_links
                .iter()
                .find(|link| link.from_entity_id == aircraft_1)
                .unwrap()
                .available
        );
        assert!(
            !projection
                .communication_links
                .iter()
                .find(|link| link.from_entity_id == aircraft_2)
                .unwrap()
                .available
        );
        assert!(matches!(
            simulation
                .transmit(aircraft_1, aircraft_2, b"jammed one to two".to_vec())
                .unwrap(),
            CommunicationOutcome::Delivered { .. }
        ));
        assert!(matches!(
            simulation
                .transmit(aircraft_2, aircraft_1, b"jammed two to one".to_vec())
                .unwrap(),
            CommunicationOutcome::Dropped {
                reason: DropReason::ReceiverInterference { .. },
                ..
            }
        ));

        for _ in 0..10 {
            simulation.step();
        }
        let projection = simulation.projection_for(aircraft_1, Side::Blue);
        assert!(projection
            .own_units
            .iter()
            .all(|unit| !unit.receiver_jammed));
        assert!(projection
            .communication_links
            .iter()
            .all(|link| link.available));
        assert!(matches!(
            simulation
                .transmit(aircraft_2, aircraft_1, b"recovered".to_vec())
                .unwrap(),
            CommunicationOutcome::Delivered { .. }
        ));
    }

    #[test]
    fn scenario_rejects_an_entity_without_a_network_device() {
        let mut scenario = jammed_flight_scenario();
        scenario.units[0].network_device_ids.clear();
        assert!(matches!(
            scenario.validate(),
            Err(ScenarioError::InvalidSimulation(message))
                if message.contains("has no c3mesh devices")
        ));
    }
}
