//! Deterministic, server-authoritative primitives for World At War.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bevy_ecs::prelude::*;
use c3mesh::{
    ChannelId, DeviceId, DeviceKind, DropReason, FrequencyBand, NetworkConfig, NetworkEvent,
    ReceiverInterference, SimTime as NetworkTime, Simulator as NetworkSimulator,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const TICK_SECONDS: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Blue,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Air,
    Land,
    Sea,
    Undersea,
    Space,
    Cyber,
}

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SimEntityId(pub Uuid);

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Ownership(pub Side);

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DomainKind(pub Domain);

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GeoPose {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub altitude_m: f64,
}

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Velocity {
    pub north_mps: f64,
    pub east_mps: f64,
    pub climb_mps: f64,
}

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Sensor {
    pub range_m: f64,
    pub identification_range_m: f64,
}

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct PlatformName(pub String);

#[derive(Component, Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSidc(pub String);

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AuthorityNode {
    pub echelon: u8,
    pub can_order: bool,
}

pub const ACTION_MOVE: &str = "move";
pub const ACTION_ENGAGE: &str = "engage";
pub const ACTION_SPACE_SUPPORT: &str = "space_support";
pub const ACTION_STRATEGIC_SATELLITE: &str = "strategic_satellite_maneuver";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRoleKind {
    Pilot,
    NationalCommand,
    DefenseSecretary,
    JointStaff,
    CombatantCommander,
    JointForceCommander,
    ComponentCommander,
    SubordinateCommander,
    TacticalCommander,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightWaypoint {
    pub at_tick: u64,
    pub position: GeoPose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CyclicFlightPath {
    pub period_ticks: u64,
    pub waypoints: Vec<FlightWaypoint>,
}

#[derive(Component, Debug, Clone)]
struct CyclicFlightPathState {
    path: CyclicFlightPath,
    active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JammingRegion {
    pub id: String,
    pub name: String,
    pub center: GeoPose,
    pub radius_m: f64,
    pub band: FrequencyBand,
    pub jammed: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationLinkDefinition {
    pub id: String,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
    pub source_device_id: DeviceId,
    pub destination_device_id: DeviceId,
    pub channel_id: ChannelId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationsConfig {
    pub network: NetworkConfig,
    pub links: Vec<CommunicationLinkDefinition>,
    pub jamming_regions: Vec<JammingRegion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRelationshipKind {
    NationalCommand,
    Cocom,
    Opcon,
    Tacon,
    Adcon,
    Support,
    Advisory,
    Transmit,
}

impl AuthorityRelationshipKind {
    pub fn is_operational(self) -> bool {
        matches!(
            self,
            Self::NationalCommand | Self::Cocom | Self::Opcon | Self::Tacon
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityRoleDefinition {
    pub id: Uuid,
    pub name: String,
    pub side: Side,
    pub kind: AuthorityRoleKind,
    pub location_unit_id: Uuid,
    pub claimable: bool,
    pub ai_controlled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityRelationship {
    pub id: Uuid,
    pub superior_role_id: Uuid,
    pub subordinate_role_id: Option<Uuid>,
    pub subordinate_unit_id: Option<Uuid>,
    pub kind: AuthorityRelationshipKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityDecisionStep {
    pub role_id: Uuid,
    pub vacant_delay_ticks: u64,
    pub approve_probability_bps: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityPolicy {
    pub id: Uuid,
    pub name: String,
    pub action: String,
    pub target_unit_ids: Vec<Uuid>,
    pub direct_role_ids: Vec<Uuid>,
    pub request_role_ids: Vec<Uuid>,
    pub decision_steps: Vec<AuthorityDecisionStep>,
    #[serde(default)]
    pub notify_role_ids: Vec<Uuid>,
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorityDefinition {
    pub version: u64,
    pub roles: Vec<AuthorityRoleDefinition>,
    pub relationships: Vec<AuthorityRelationship>,
    pub policies: Vec<AuthorityPolicy>,
}

impl AuthorityDefinition {
    pub fn validate(&self, unit_ids: &BTreeSet<Uuid>) -> Result<(), String> {
        let roles: BTreeMap<_, _> = self.roles.iter().map(|role| (role.id, role)).collect();
        if roles.len() != self.roles.len() {
            return Err("authority definition contains duplicate role ids".into());
        }
        for role in &self.roles {
            if role.name.trim().is_empty() || !unit_ids.contains(&role.location_unit_id) {
                return Err(format!("role {} has an invalid name or location", role.id));
            }
        }
        let mut operational_parent: BTreeMap<Uuid, Uuid> = BTreeMap::new();
        let mut unit_parent: BTreeMap<Uuid, Uuid> = BTreeMap::new();
        for edge in &self.relationships {
            if !roles.contains_key(&edge.superior_role_id)
                || edge.subordinate_role_id.is_some() == edge.subordinate_unit_id.is_some()
            {
                return Err(format!("relationship {} has invalid endpoints", edge.id));
            }
            if let Some(role_id) = edge.subordinate_role_id {
                if !roles.contains_key(&role_id) {
                    return Err(format!(
                        "relationship {} references a missing role",
                        edge.id
                    ));
                }
                if edge.kind.is_operational()
                    && operational_parent
                        .insert(role_id, edge.superior_role_id)
                        .is_some()
                {
                    return Err(format!("role {role_id} has multiple operational parents"));
                }
            }
            if let Some(unit_id) = edge.subordinate_unit_id {
                if !unit_ids.contains(&unit_id) {
                    return Err(format!(
                        "relationship {} references a missing unit",
                        edge.id
                    ));
                }
                if edge.kind.is_operational()
                    && unit_parent.insert(unit_id, edge.superior_role_id).is_some()
                {
                    return Err(format!("unit {unit_id} has multiple operational parents"));
                }
            }
        }
        for role_id in roles.keys() {
            let mut seen = BTreeSet::new();
            let mut cursor = *role_id;
            while let Some(parent) = operational_parent.get(&cursor) {
                if !seen.insert(cursor) {
                    return Err(format!("operational command cycle includes role {cursor}"));
                }
                cursor = *parent;
            }
        }
        for unit_id in unit_ids {
            let Some(mut cursor) = unit_parent.get(unit_id).copied() else {
                return Err(format!("unit {unit_id} has no operational command parent"));
            };
            let mut seen = BTreeSet::new();
            while let Some(parent) = operational_parent.get(&cursor) {
                if !seen.insert(cursor) {
                    return Err(format!("operational command cycle reaches unit {unit_id}"));
                }
                cursor = *parent;
            }
            let root = roles
                .get(&cursor)
                .ok_or_else(|| format!("unit {unit_id} has no command root"))?;
            if root.kind != AuthorityRoleKind::NationalCommand
                && !(root.kind == AuthorityRoleKind::Pilot && root.location_unit_id == *unit_id)
            {
                return Err(format!(
                    "unit {unit_id} does not terminate at national command or its colocated pilot"
                ));
            }
        }
        for policy in &self.policies {
            if policy.name.trim().is_empty()
                || policy.action.trim().is_empty()
                || policy.target_unit_ids.is_empty()
            {
                return Err(format!("policy {} is incomplete", policy.id));
            }
            if policy
                .target_unit_ids
                .iter()
                .any(|id| !unit_ids.contains(id))
            {
                return Err(format!("policy {} references a missing unit", policy.id));
            }
            for role_id in policy
                .direct_role_ids
                .iter()
                .chain(&policy.request_role_ids)
                .chain(&policy.notify_role_ids)
            {
                if !roles.contains_key(role_id) {
                    return Err(format!("policy {} references a missing role", policy.id));
                }
            }
            for step in &policy.decision_steps {
                if !roles.contains_key(&step.role_id) || step.approve_probability_bps > 10_000 {
                    return Err(format!("policy {} has an invalid decision step", policy.id));
                }
            }
            for role_id in &policy.direct_role_ids {
                if policy
                    .target_unit_ids
                    .iter()
                    .any(|unit_id| !self.role_is_in_unit_chain(*role_id, *unit_id))
                {
                    return Err(format!(
                        "policy {} grants direct authority outside the target chain",
                        policy.id
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn role_is_in_unit_chain(&self, role_id: Uuid, unit_id: Uuid) -> bool {
        let role_parents: BTreeMap<_, _> = self
            .relationships
            .iter()
            .filter(|edge| edge.kind.is_operational())
            .filter_map(|edge| {
                edge.subordinate_role_id
                    .map(|child| (child, edge.superior_role_id))
            })
            .collect();
        let mut cursor = self
            .relationships
            .iter()
            .find(|edge| edge.kind.is_operational() && edge.subordinate_unit_id == Some(unit_id))
            .map(|edge| edge.superior_role_id);
        while let Some(current) = cursor {
            if current == role_id {
                return true;
            }
            cursor = role_parents.get(&current).copied();
        }
        false
    }

    pub fn controlled_units(&self, role_id: Uuid) -> Vec<Uuid> {
        let mut units = BTreeSet::new();
        let mut descendants = BTreeSet::from([role_id]);
        loop {
            let before = descendants.len();
            for edge in self
                .relationships
                .iter()
                .filter(|edge| edge.kind.is_operational())
            {
                if descendants.contains(&edge.superior_role_id) {
                    if let Some(child) = edge.subordinate_role_id {
                        descendants.insert(child);
                    }
                    if let Some(unit) = edge.subordinate_unit_id {
                        units.insert(unit);
                    }
                }
            }
            if descendants.len() == before {
                break;
            }
        }
        units.into_iter().collect()
    }

    pub fn policy_for(&self, action: &str, target: Uuid) -> Option<&AuthorityPolicy> {
        self.policies
            .iter()
            .find(|policy| policy.action == action && policy.target_unit_ids.contains(&target))
    }
}

#[derive(Resource, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SimClock {
    pub tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub observer: Uuid,
    pub target: Uuid,
    pub side: Side,
    pub observed_tick: u64,
    pub position: GeoPose,
    pub identity_confidence: f32,
}

#[derive(Resource, Debug, Default)]
pub struct Observations(pub Vec<Contact>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub track_id: Uuid,
    pub target_side: Side,
    pub position: GeoPose,
    pub identity_confidence: f32,
    pub observed_tick: u64,
    pub received_tick: u64,
    pub observed_sidc: String,
}

#[derive(Resource, Debug, Default)]
pub struct KnowledgeBases(pub BTreeMap<Uuid, Vec<Track>>);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderKind {
    Move { north_mps: f64, east_mps: f64 },
    Engage { track_id: Uuid },
}

impl OrderKind {
    pub fn action_key(&self) -> &'static str {
        match self {
            Self::Move { .. } => ACTION_MOVE,
            Self::Engage { .. } => ACTION_ENGAGE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerIntent {
    pub intent_id: Uuid,
    pub issuer_role: Uuid,
    pub target: Uuid,
    pub kind: OrderKind,
    pub requested_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationRecord {
    pub policy_id: Uuid,
    pub policy_version: u64,
    pub requester_role_id: Uuid,
    pub granting_role_id: Uuid,
    pub request_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizedIntent {
    pub intent: PlayerIntent,
    pub authorization: AuthorizationRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSpawn {
    pub id: Uuid,
    pub name: String,
    pub side: Side,
    pub domain: Domain,
    pub pose: GeoPose,
    pub velocity: Velocity,
    pub sensor: Option<Sensor>,
    pub network_device_ids: Vec<DeviceId>,
    pub flight_path: Option<CyclicFlightPath>,
    pub sidc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderStatus {
    Accepted,
    Rejected(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResult {
    pub intent_id: Uuid,
    pub status: OrderStatus,
}

#[derive(Resource, Debug, Default)]
pub struct PendingIntents(pub VecDeque<AuthorizedIntent>);

#[derive(Resource, Debug, Default)]
pub struct OrderResults(pub Vec<OrderResult>);

#[derive(Debug, Error)]
pub enum SimulationBuildError {
    #[error("invalid simulation configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid c3mesh network: {0}")]
    Network(#[from] c3mesh::SimulationError),
}

#[derive(Debug, Error)]
pub enum CommunicationError {
    #[error("no configured communication link from {from} to {to}")]
    NoLink { from: Uuid, to: Uuid },
    #[error("c3mesh communication failed: {0}")]
    Network(#[from] c3mesh::SimulationError),
    #[error("c3mesh became idle before packet {0} reached a terminal state")]
    MissingTerminalEvent(u64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommunicationOutcome {
    Delivered { at_ns: u64 },
    Dropped { at_ns: u64, reason: DropReason },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationLinkStatus {
    pub id: String,
    pub from_entity_id: Uuid,
    pub to_entity_id: Uuid,
    pub available: bool,
    pub jammed: f64,
    pub effective_bit_rate_bps: Option<u64>,
}

struct CommunicationsRuntime {
    simulator: NetworkSimulator,
    entity_devices: BTreeMap<Uuid, Vec<DeviceId>>,
    baseline_interference: BTreeMap<DeviceId, Vec<ReceiverInterference>>,
    links: Vec<CommunicationLinkDefinition>,
    jamming_regions: Vec<JammingRegion>,
}

pub struct Simulation {
    world: World,
    schedule: Schedule,
    communications: CommunicationsRuntime,
}

impl Simulation {
    pub fn validate_configuration(
        platforms: &[PlatformSpawn],
        communications: &CommunicationsConfig,
    ) -> Result<(), SimulationBuildError> {
        communications
            .network
            .validate()
            .map_err(|error| SimulationBuildError::InvalidConfiguration(error.to_string()))?;

        let mut platform_ids = BTreeSet::new();
        let devices: BTreeMap<_, _> = communications
            .network
            .devices
            .iter()
            .map(|device| (device.id.clone(), device))
            .collect();
        let mut device_owners = BTreeMap::new();
        for platform in platforms {
            if !platform_ids.insert(platform.id) {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "duplicate platform id {}",
                    platform.id
                )));
            }
            if platform.network_device_ids.is_empty() {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "platform {} has no c3mesh devices",
                    platform.id
                )));
            }
            if let Some(path) = &platform.flight_path {
                validate_flight_path(platform.id, path)?;
            }
            for device_id in &platform.network_device_ids {
                if !devices.contains_key(device_id) {
                    return Err(SimulationBuildError::InvalidConfiguration(format!(
                        "platform {} references unknown device {}",
                        platform.id, device_id
                    )));
                }
                if device_owners
                    .insert(device_id.clone(), platform.id)
                    .is_some()
                {
                    return Err(SimulationBuildError::InvalidConfiguration(format!(
                        "device {device_id} is assigned to multiple platforms"
                    )));
                }
            }
        }
        if device_owners.len() != devices.len() {
            let unowned = devices
                .keys()
                .find(|device| !device_owners.contains_key(*device))
                .expect("different device counts imply an unowned device");
            return Err(SimulationBuildError::InvalidConfiguration(format!(
                "device {unowned} is not assigned to a platform"
            )));
        }

        let channels: BTreeMap<_, _> = communications
            .network
            .channels
            .iter()
            .map(|channel| (channel.id.clone(), channel))
            .collect();
        let mut link_ids = BTreeSet::new();
        let mut entity_pairs = BTreeSet::new();
        for link in &communications.links {
            if link.id.trim().is_empty() || !link_ids.insert(link.id.clone()) {
                return Err(SimulationBuildError::InvalidConfiguration(
                    "communication link ids must be non-empty and unique".into(),
                ));
            }
            if !entity_pairs.insert((link.from_entity_id, link.to_entity_id)) {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "multiple default links are configured from {} to {}",
                    link.from_entity_id, link.to_entity_id
                )));
            }
            if device_owners.get(&link.source_device_id) != Some(&link.from_entity_id)
                || device_owners.get(&link.destination_device_id) != Some(&link.to_entity_id)
            {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "link {} device ownership does not match its entities",
                    link.id
                )));
            }
            let Some(source) = devices.get(&link.source_device_id) else {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "link {} references an unknown source",
                    link.id
                )));
            };
            let Some(destination) = devices.get(&link.destination_device_id) else {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "link {} references an unknown destination",
                    link.id
                )));
            };
            if !matches!(destination.kind, DeviceKind::Sink)
                || !matches!(
                    &source.kind,
                    DeviceKind::Source { egress } if egress == &link.channel_id
                )
            {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "link {} must connect a source's egress to a sink",
                    link.id
                )));
            }
            let Some(channel) = channels.get(&link.channel_id) else {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "link {} references an unknown channel",
                    link.id
                )));
            };
            if !channel.endpoints.contains(&link.source_device_id)
                || !channel.endpoints.contains(&link.destination_device_id)
            {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "link {} devices are not the endpoints of channel {}",
                    link.id, link.channel_id
                )));
            }
        }

        let mut region_ids = BTreeSet::new();
        for region in &communications.jamming_regions {
            if region.id.trim().is_empty()
                || !region_ids.insert(region.id.clone())
                || !geo_pose_is_finite(region.center)
                || !region.radius_m.is_finite()
                || region.radius_m <= 0.0
                || region.band.lower_hz >= region.band.upper_hz
                || !region.jammed.is_finite()
                || !(0.0..=1.0).contains(&region.jammed)
            {
                return Err(SimulationBuildError::InvalidConfiguration(format!(
                    "jamming region {} is invalid",
                    region.id
                )));
            }
        }
        Ok(())
    }

    pub fn new(
        platforms: Vec<PlatformSpawn>,
        communications: CommunicationsConfig,
    ) -> Result<Self, SimulationBuildError> {
        Self::validate_configuration(&platforms, &communications)?;
        let entity_devices = platforms
            .iter()
            .map(|platform| (platform.id, platform.network_device_ids.clone()))
            .collect();
        let baseline_interference = communications
            .network
            .devices
            .iter()
            .map(|device| (device.id.clone(), device.interference.clone()))
            .collect();
        let simulator = NetworkSimulator::new(communications.network)?;
        let mut world = World::new();
        world.insert_resource(SimClock { tick: 0 });
        world.insert_resource(Observations::default());
        world.insert_resource(KnowledgeBases::default());
        world.insert_resource(PendingIntents::default());
        world.insert_resource(OrderResults::default());

        let mut schedule = Schedule::default();
        schedule.add_systems((
            advance_clock,
            apply_orders.after(advance_clock),
            move_platforms.after(apply_orders),
            detect_contacts.after(move_platforms),
            deliver_reports.after(detect_contacts),
        ));
        let mut simulation = Self {
            world,
            schedule,
            communications: CommunicationsRuntime {
                simulator,
                entity_devices,
                baseline_interference,
                links: communications.links,
                jamming_regions: communications.jamming_regions,
            },
        };
        for platform in platforms {
            simulation.spawn_platform(platform);
        }
        simulation.sync_network_interference()?;
        Ok(simulation)
    }

    fn spawn_platform(&mut self, platform: PlatformSpawn) {
        let mut entity = self.world.spawn((
            SimEntityId(platform.id),
            PlatformName(platform.name),
            PlatformSidc(platform.sidc),
            Ownership(platform.side),
            DomainKind(platform.domain),
            platform.pose,
            platform.velocity,
            AuthorityNode {
                echelon: 1,
                can_order: true,
            },
        ));
        if let Some(sensor) = platform.sensor {
            entity.insert(sensor);
        }
        if let Some(path) = platform.flight_path {
            entity.insert(CyclicFlightPathState { path, active: true });
        }
    }

    pub fn queue_authorized_intent(&mut self, intent: AuthorizedIntent) {
        self.world
            .resource_mut::<PendingIntents>()
            .0
            .push_back(intent);
    }

    pub fn step(&mut self) {
        self.schedule.run(&mut self.world);
        self.sync_network_interference()
            .expect("validated network must accept tick interference");
    }

    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    pub fn drain_order_results(&mut self) -> Vec<OrderResult> {
        std::mem::take(&mut self.world.resource_mut::<OrderResults>().0)
    }

    pub fn transmit(
        &mut self,
        from_entity_id: Uuid,
        to_entity_id: Uuid,
        payload: Vec<u8>,
    ) -> Result<CommunicationOutcome, CommunicationError> {
        let link = self
            .communications
            .links
            .iter()
            .find(|link| link.from_entity_id == from_entity_id && link.to_entity_id == to_entity_id)
            .cloned()
            .ok_or(CommunicationError::NoLink {
                from: from_entity_id,
                to: to_entity_id,
            })?;
        let at = self.network_time();
        let packet_id = self.communications.simulator.schedule_send(
            at,
            link.source_device_id,
            link.destination_device_id,
            payload,
        )?;
        while let Some(event) = self.communications.simulator.step()? {
            match event {
                NetworkEvent::PacketDelivered { at, packet, .. } if packet.id() == packet_id => {
                    return Ok(CommunicationOutcome::Delivered {
                        at_ns: at.as_nanos(),
                    });
                }
                NetworkEvent::PacketDropped {
                    at, packet, reason, ..
                } if packet.id() == packet_id => {
                    return Ok(CommunicationOutcome::Dropped {
                        at_ns: at.as_nanos(),
                        reason,
                    });
                }
                _ => {}
            }
        }
        Err(CommunicationError::MissingTerminalEvent(packet_id.get()))
    }

    /// Builds a projection from the knowledge held by the role's assigned command node.
    /// Higher-echelon aggregation is added by the communications system, not by visibility code.
    pub fn projection_for(&mut self, knowledge_owner: Uuid, side: Side) -> RoleProjection {
        let tracks = self
            .world
            .resource::<KnowledgeBases>()
            .0
            .get(&knowledge_owner)
            .cloned()
            .unwrap_or_default();
        let link_statuses = self.communication_link_statuses();
        let network_time = self.network_time();
        let receiver_jammed: BTreeMap<_, _> = self
            .communications
            .entity_devices
            .keys()
            .map(|entity_id| {
                (
                    *entity_id,
                    self.entity_receiver_jammed(*entity_id, network_time),
                )
            })
            .collect();
        let mut own_units = Vec::new();
        let mut query = self.world.query::<(
            &SimEntityId,
            &PlatformName,
            &Ownership,
            &DomainKind,
            &GeoPose,
            &PlatformSidc,
        )>();
        for (id, name, ownership, domain, pose, sidc) in query.iter(&self.world) {
            if ownership.0 == side {
                own_units.push(VisibleUnit {
                    id: id.0,
                    name: name.0.clone(),
                    domain: domain.0,
                    position: *pose,
                    sidc: sidc.0.clone(),
                    receiver_jammed: receiver_jammed.get(&id.0).copied().unwrap_or(false),
                });
            }
        }
        RoleProjection {
            tick: self.tick(),
            own_units,
            tracks,
            jamming_regions: self.communications.jamming_regions.clone(),
            communication_links: link_statuses,
        }
    }

    fn network_time(&self) -> NetworkTime {
        let tick_ns = self
            .tick()
            .saturating_mul(TICK_SECONDS)
            .saturating_mul(1_000_000_000);
        NetworkTime::from_nanos(tick_ns.max(self.communications.simulator.now().as_nanos()))
    }

    fn sync_network_interference(&mut self) -> Result<(), c3mesh::SimulationError> {
        let at = self.network_time();
        let mut query = self.world.query::<(&SimEntityId, &GeoPose)>();
        let positions: BTreeMap<_, _> = query
            .iter(&self.world)
            .map(|(id, pose)| (id.0, *pose))
            .collect();
        for (entity_id, device_ids) in &self.communications.entity_devices {
            let Some(position) = positions.get(entity_id) else {
                continue;
            };
            for device_id in device_ids {
                let mut snapshot = self
                    .communications
                    .baseline_interference
                    .get(device_id)
                    .cloned()
                    .unwrap_or_default();
                for region in &self.communications.jamming_regions {
                    if great_circle_distance_m(*position, region.center) <= region.radius_m {
                        snapshot.push(ReceiverInterference {
                            band: region.band,
                            jammed: region.jammed,
                        });
                    }
                }
                self.communications
                    .simulator
                    .schedule_receiver_interference(at, device_id.clone(), snapshot)?;
            }
        }
        Ok(())
    }

    fn entity_receiver_jammed(&self, entity_id: Uuid, at: NetworkTime) -> bool {
        self.communications
            .entity_devices
            .get(&entity_id)
            .into_iter()
            .flatten()
            .any(|device_id| {
                self.communications
                    .simulator
                    .receiver_interference_at(device_id.clone(), at)
                    .is_ok_and(|snapshot| snapshot.iter().any(|item| item.jammed > 0.0))
            })
    }

    fn communication_link_statuses(&self) -> Vec<CommunicationLinkStatus> {
        let at = self.network_time();
        self.communications
            .links
            .iter()
            .map(|link| {
                let metrics = self
                    .communications
                    .simulator
                    .transmission_metrics_at(
                        link.channel_id.clone(),
                        link.source_device_id.clone(),
                        at,
                    )
                    .expect("validated monitored link must remain queryable");
                CommunicationLinkStatus {
                    id: link.id.clone(),
                    from_entity_id: link.from_entity_id,
                    to_entity_id: link.to_entity_id,
                    available: metrics.available,
                    jammed: if metrics.jammed == 0.0 {
                        0.0
                    } else {
                        metrics.jammed
                    },
                    effective_bit_rate_bps: metrics.effective_bit_rate_bps,
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisibleUnit {
    pub id: Uuid,
    pub name: String,
    pub domain: Domain,
    pub position: GeoPose,
    pub sidc: String,
    pub receiver_jammed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleProjection {
    pub tick: u64,
    pub own_units: Vec<VisibleUnit>,
    pub tracks: Vec<Track>,
    pub jamming_regions: Vec<JammingRegion>,
    pub communication_links: Vec<CommunicationLinkStatus>,
}

fn advance_clock(mut clock: ResMut<SimClock>) {
    clock.tick += 1;
}

fn apply_orders(
    clock: Res<SimClock>,
    mut pending: ResMut<PendingIntents>,
    mut results: ResMut<OrderResults>,
    mut units: Query<(
        &SimEntityId,
        &AuthorityNode,
        &mut Velocity,
        Option<&mut CyclicFlightPathState>,
    )>,
) {
    let mut deferred = VecDeque::new();
    while let Some(authorized) = pending.0.pop_front() {
        let intent = authorized.intent;
        if intent.requested_tick > clock.tick {
            deferred.push_back(AuthorizedIntent {
                intent,
                authorization: authorized.authorization,
            });
            continue;
        }
        let Some((_, authority, mut velocity, mut flight_path)) =
            units.iter_mut().find(|(id, _, _, _)| id.0 == intent.target)
        else {
            results.0.push(OrderResult {
                intent_id: intent.intent_id,
                status: OrderStatus::Rejected("target does not exist".into()),
            });
            continue;
        };
        if !authority.can_order {
            results.0.push(OrderResult {
                intent_id: intent.intent_id,
                status: OrderStatus::Rejected("target cannot accept orders".into()),
            });
            continue;
        }
        match intent.kind {
            OrderKind::Move {
                north_mps,
                east_mps,
            } => {
                velocity.north_mps = north_mps;
                velocity.east_mps = east_mps;
                if let Some(path) = flight_path.as_deref_mut() {
                    path.active = false;
                }
                results.0.push(OrderResult {
                    intent_id: intent.intent_id,
                    status: OrderStatus::Accepted,
                });
            }
            OrderKind::Engage { .. } => results.0.push(OrderResult {
                intent_id: intent.intent_id,
                status: OrderStatus::Rejected(
                    "engagement modelling is not available in the training slice".into(),
                ),
            }),
        }
    }
    pending.0 = deferred;
}

fn move_platforms(
    clock: Res<SimClock>,
    mut units: Query<(&mut GeoPose, &Velocity, Option<&CyclicFlightPathState>)>,
) {
    for (mut pose, velocity, flight_path) in &mut units {
        if let Some(path) = flight_path.filter(|path| path.active) {
            *pose = position_on_flight_path(&path.path, clock.tick);
            continue;
        }
        pose.latitude_deg += velocity.north_mps * TICK_SECONDS as f64 / 111_320.0;
        let longitude_scale = 111_320.0 * pose.latitude_deg.to_radians().cos().abs().max(0.01);
        pose.longitude_deg += velocity.east_mps * TICK_SECONDS as f64 / longitude_scale;
        pose.altitude_m = (pose.altitude_m + velocity.climb_mps * TICK_SECONDS as f64).max(0.0);
    }
}

fn validate_flight_path(
    platform_id: Uuid,
    path: &CyclicFlightPath,
) -> Result<(), SimulationBuildError> {
    let valid = path.period_ticks > 0
        && path.waypoints.len() >= 2
        && path
            .waypoints
            .first()
            .is_some_and(|waypoint| waypoint.at_tick == 0)
        && path
            .waypoints
            .windows(2)
            .all(|pair| pair[0].at_tick < pair[1].at_tick)
        && path
            .waypoints
            .last()
            .is_some_and(|waypoint| waypoint.at_tick < path.period_ticks)
        && path
            .waypoints
            .iter()
            .all(|waypoint| geo_pose_is_finite(waypoint.position));
    if valid {
        Ok(())
    } else {
        Err(SimulationBuildError::InvalidConfiguration(format!(
            "platform {platform_id} has an invalid cyclic flight path"
        )))
    }
}

fn position_on_flight_path(path: &CyclicFlightPath, tick: u64) -> GeoPose {
    let cycle_tick = tick % path.period_ticks;
    let upper_index = path
        .waypoints
        .partition_point(|waypoint| waypoint.at_tick <= cycle_tick);
    let (lower, upper, upper_tick) = if upper_index < path.waypoints.len() {
        (
            &path.waypoints[upper_index - 1],
            &path.waypoints[upper_index],
            path.waypoints[upper_index].at_tick,
        )
    } else {
        (
            path.waypoints.last().expect("validated path has waypoints"),
            path.waypoints
                .first()
                .expect("validated path has waypoints"),
            path.period_ticks,
        )
    };
    let fraction = (cycle_tick - lower.at_tick) as f64 / (upper_tick - lower.at_tick) as f64;
    GeoPose {
        latitude_deg: lower.position.latitude_deg
            + (upper.position.latitude_deg - lower.position.latitude_deg) * fraction,
        longitude_deg: lower.position.longitude_deg
            + (upper.position.longitude_deg - lower.position.longitude_deg) * fraction,
        altitude_m: lower.position.altitude_m
            + (upper.position.altitude_m - lower.position.altitude_m) * fraction,
    }
}

fn geo_pose_is_finite(position: GeoPose) -> bool {
    position.latitude_deg.is_finite()
        && position.longitude_deg.is_finite()
        && position.altitude_m.is_finite()
        && (-90.0..=90.0).contains(&position.latitude_deg)
        && (-180.0..=180.0).contains(&position.longitude_deg)
}

fn detect_contacts(
    clock: Res<SimClock>,
    mut observations: ResMut<Observations>,
    sensors: Query<(&SimEntityId, &Ownership, &GeoPose, &Sensor)>,
    targets: Query<(&SimEntityId, &Ownership, &GeoPose)>,
) {
    observations.0.clear();
    for (observer_id, observer_side, observer_pose, sensor) in &sensors {
        for (target_id, target_side, target_pose) in &targets {
            if observer_side.0 == target_side.0 {
                continue;
            }
            let range = great_circle_distance_m(*observer_pose, *target_pose);
            if range <= sensor.range_m {
                observations.0.push(Contact {
                    observer: observer_id.0,
                    target: target_id.0,
                    side: target_side.0,
                    observed_tick: clock.tick,
                    position: *target_pose,
                    identity_confidence: if range <= sensor.identification_range_m {
                        0.9
                    } else {
                        0.45
                    },
                });
            }
        }
    }
}

fn deliver_reports(
    clock: Res<SimClock>,
    observations: Res<Observations>,
    mut knowledge: ResMut<KnowledgeBases>,
) {
    for contact in &observations.0 {
        let tracks = knowledge.0.entry(contact.observer).or_default();
        if let Some(track) = tracks
            .iter_mut()
            .find(|track| track.track_id == contact.target)
        {
            track.position = contact.position;
            track.identity_confidence = contact.identity_confidence;
            track.observed_tick = contact.observed_tick;
            track.received_tick = clock.tick;
        } else {
            tracks.push(Track {
                track_id: contact.target,
                target_side: contact.side,
                position: contact.position,
                identity_confidence: contact.identity_confidence,
                observed_tick: contact.observed_tick,
                received_tick: clock.tick,
                observed_sidc: unknown_sidc(contact.side).into(),
            });
        }
    }
}

fn unknown_sidc(side: Side) -> &'static str {
    match side {
        Side::Blue => "100301000000000000000000000000",
        Side::Red => "100601000000000000000000000000",
    }
}

fn great_circle_distance_m(a: GeoPose, b: GeoPose) -> f64 {
    let earth_radius_m = 6_371_000.0;
    let d_lat = (b.latitude_deg - a.latitude_deg).to_radians();
    let d_lon = (b.longitude_deg - a.longitude_deg).to_radians();
    let h = (d_lat / 2.0).sin().powi(2)
        + a.latitude_deg.to_radians().cos()
            * b.latitude_deg.to_radians().cos()
            * (d_lon / 2.0).sin().powi(2);
    2.0 * earth_radius_m * h.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device_id(id: Uuid) -> DeviceId {
        DeviceId::new(format!("test-{id}-network"))
    }

    fn test_simulation(platforms: Vec<PlatformSpawn>) -> Simulation {
        let devices = platforms
            .iter()
            .flat_map(|platform| platform.network_device_ids.iter().cloned())
            .map(|id| c3mesh::DeviceConfig {
                id,
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
                links: vec![],
                jamming_regions: vec![],
            },
        )
        .unwrap()
    }

    #[test]
    fn detects_unit_when_inside_sensor_range() {
        let blue = Uuid::new_v4();
        let red = Uuid::new_v4();
        let mut sim = test_simulation(vec![
            PlatformSpawn {
                id: blue,
                name: "Blue sensor".into(),
                side: Side::Blue,
                domain: Domain::Air,
                pose: GeoPose {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0,
                    altitude_m: 1000.0,
                },
                velocity: Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                },
                sensor: Some(Sensor {
                    range_m: 20_000.0,
                    identification_range_m: 5_000.0,
                }),
                network_device_ids: vec![test_device_id(blue)],
                flight_path: None,
                sidc: "100301000011010000000000000000".into(),
            },
            PlatformSpawn {
                id: red,
                name: "Red target".into(),
                side: Side::Red,
                domain: Domain::Air,
                pose: GeoPose {
                    latitude_deg: 0.05,
                    longitude_deg: 0.0,
                    altitude_m: 1000.0,
                },
                velocity: Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                },
                sensor: None,
                network_device_ids: vec![test_device_id(red)],
                flight_path: None,
                sidc: "100601000011010000000000000000".into(),
            },
        ]);
        sim.step();
        let projection = sim.projection_for(blue, Side::Blue);
        assert_eq!(projection.tracks.len(), 1);
        assert_eq!(projection.tracks[0].observed_sidc.len(), 30);
        assert_ne!(
            projection.tracks[0].observed_sidc,
            "100601000011010000000000000000"
        );
    }

    #[test]
    fn projection_does_not_include_enemy_units_without_tracks() {
        let blue = Uuid::new_v4();
        let red = Uuid::new_v4();
        let mut sim = test_simulation(vec![
            PlatformSpawn {
                id: blue,
                name: "Blue".into(),
                side: Side::Blue,
                domain: Domain::Land,
                pose: GeoPose {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0,
                    altitude_m: 0.0,
                },
                velocity: Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                },
                sensor: None,
                network_device_ids: vec![test_device_id(blue)],
                flight_path: None,
                sidc: "100310000012110000000000000000".into(),
            },
            PlatformSpawn {
                id: red,
                name: "Red".into(),
                side: Side::Red,
                domain: Domain::Land,
                pose: GeoPose {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0,
                    altitude_m: 0.0,
                },
                velocity: Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                },
                sensor: None,
                network_device_ids: vec![test_device_id(red)],
                flight_path: None,
                sidc: "100610000012110000000000000000".into(),
            },
        ]);
        assert_eq!(sim.projection_for(blue, Side::Blue).own_units.len(), 1);
        assert!(sim.projection_for(blue, Side::Blue).tracks.is_empty());
    }

    fn authority_fixture() -> (AuthorityDefinition, Uuid, Uuid, Uuid) {
        let root = Uuid::from_u128(1);
        let commander = Uuid::from_u128(2);
        let unit = Uuid::from_u128(3);
        let definition = AuthorityDefinition {
            version: 1,
            roles: vec![
                AuthorityRoleDefinition {
                    id: root,
                    name: "National Command".into(),
                    side: Side::Blue,
                    kind: AuthorityRoleKind::NationalCommand,
                    location_unit_id: unit,
                    claimable: true,
                    ai_controlled: false,
                },
                AuthorityRoleDefinition {
                    id: commander,
                    name: "Commander".into(),
                    side: Side::Blue,
                    kind: AuthorityRoleKind::ComponentCommander,
                    location_unit_id: unit,
                    claimable: true,
                    ai_controlled: false,
                },
            ],
            relationships: vec![
                AuthorityRelationship {
                    id: Uuid::from_u128(10),
                    superior_role_id: root,
                    subordinate_role_id: Some(commander),
                    subordinate_unit_id: None,
                    kind: AuthorityRelationshipKind::Opcon,
                },
                AuthorityRelationship {
                    id: Uuid::from_u128(11),
                    superior_role_id: commander,
                    subordinate_role_id: None,
                    subordinate_unit_id: Some(unit),
                    kind: AuthorityRelationshipKind::Tacon,
                },
            ],
            policies: vec![AuthorityPolicy {
                id: Uuid::from_u128(20),
                name: "Movement".into(),
                action: ACTION_MOVE.into(),
                target_unit_ids: vec![unit],
                direct_role_ids: vec![root, commander],
                request_role_ids: vec![],
                decision_steps: vec![],
                notify_role_ids: vec![],
                executable: true,
            }],
        };
        (definition, root, commander, unit)
    }

    #[test]
    fn authority_definition_requires_a_national_root_and_rejects_cycles() {
        let (mut definition, _, commander, unit) = authority_fixture();
        definition.validate(&BTreeSet::from([unit])).unwrap();
        definition.relationships.push(AuthorityRelationship {
            id: Uuid::from_u128(12),
            superior_role_id: commander,
            subordinate_role_id: Some(Uuid::from_u128(1)),
            subordinate_unit_id: None,
            kind: AuthorityRelationshipKind::Opcon,
        });
        assert!(definition
            .validate(&BTreeSet::from([unit]))
            .unwrap_err()
            .contains("cycle"));
    }

    #[test]
    fn simulation_only_executes_authorized_intents() {
        let (definition, _, commander, unit) = authority_fixture();
        let mut sim = test_simulation(vec![PlatformSpawn {
            id: unit,
            name: "Unit".into(),
            side: Side::Blue,
            domain: Domain::Land,
            pose: GeoPose {
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                altitude_m: 0.0,
            },
            velocity: Velocity {
                north_mps: 0.0,
                east_mps: 0.0,
                climb_mps: 0.0,
            },
            sensor: None,
            network_device_ids: vec![test_device_id(unit)],
            flight_path: None,
            sidc: "100310000012110000000000000000".into(),
        }]);
        let intent_id = Uuid::from_u128(30);
        sim.queue_authorized_intent(AuthorizedIntent {
            intent: PlayerIntent {
                intent_id,
                issuer_role: commander,
                target: unit,
                kind: OrderKind::Move {
                    north_mps: 111.32,
                    east_mps: 0.0,
                },
                requested_tick: 1,
            },
            authorization: AuthorizationRecord {
                policy_id: definition.policies[0].id,
                policy_version: 1,
                requester_role_id: commander,
                granting_role_id: commander,
                request_id: None,
            },
        });
        sim.step();
        assert!(matches!(
            sim.drain_order_results()[0].status,
            OrderStatus::Accepted
        ));
        assert!(
            (sim.projection_for(unit, Side::Blue).own_units[0]
                .position
                .latitude_deg
                - 0.001)
                .abs()
                < 0.00001
        );
    }

    #[test]
    fn accepted_move_order_cancels_a_cyclic_flight_path() {
        let unit = Uuid::from_u128(40);
        let mut sim = test_simulation(vec![PlatformSpawn {
            id: unit,
            name: "Scripted aircraft".into(),
            side: Side::Blue,
            domain: Domain::Air,
            pose: GeoPose {
                latitude_deg: 0.0,
                longitude_deg: 0.0,
                altitude_m: 1_000.0,
            },
            velocity: Velocity {
                north_mps: 0.0,
                east_mps: 0.0,
                climb_mps: 0.0,
            },
            sensor: None,
            network_device_ids: vec![test_device_id(unit)],
            flight_path: Some(CyclicFlightPath {
                period_ticks: 20,
                waypoints: vec![
                    FlightWaypoint {
                        at_tick: 0,
                        position: GeoPose {
                            latitude_deg: 0.0,
                            longitude_deg: 0.0,
                            altitude_m: 1_000.0,
                        },
                    },
                    FlightWaypoint {
                        at_tick: 10,
                        position: GeoPose {
                            latitude_deg: 0.0,
                            longitude_deg: 10.0,
                            altitude_m: 1_000.0,
                        },
                    },
                ],
            }),
            sidc: "100301000011010000000000000000".into(),
        }]);
        sim.step();
        assert_eq!(
            sim.projection_for(unit, Side::Blue).own_units[0]
                .position
                .longitude_deg,
            1.0
        );
        sim.queue_authorized_intent(AuthorizedIntent {
            intent: PlayerIntent {
                intent_id: Uuid::from_u128(41),
                issuer_role: Uuid::from_u128(42),
                target: unit,
                kind: OrderKind::Move {
                    north_mps: 111.32,
                    east_mps: 0.0,
                },
                requested_tick: 2,
            },
            authorization: AuthorizationRecord {
                policy_id: Uuid::from_u128(43),
                policy_version: 1,
                requester_role_id: Uuid::from_u128(42),
                granting_role_id: Uuid::from_u128(42),
                request_id: None,
            },
        });
        sim.step();
        let position = sim.projection_for(unit, Side::Blue).own_units[0].position;
        assert!((position.latitude_deg - 0.001).abs() < 0.00001);
        assert_eq!(position.longitude_deg, 1.0);
    }
}
