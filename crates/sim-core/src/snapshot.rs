//! Complete, versioned simulation checkpoints. These are private server data;
//! never expose a snapshot through a role projection.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
struct PlatformState {
    id: SimEntityId,
    name: PlatformName,
    sidc: PlatformSidc,
    owner: Ownership,
    domain: DomainKind,
    pose: GeoPose,
    velocity: Velocity,
    motion: GeodesicMotionState,
    authority: AuthorityNode,
    combat: operations::CombatState,
    sensor: Option<Sensor>,
    path: Option<CyclicFlightPathState>,
}

/// A complete tick-boundary checkpoint, including in-flight network state.
#[derive(Clone, Serialize, Deserialize)]
pub struct SimulationSnapshot {
    schema_version: u32,
    platforms: Vec<PlatformState>,
    clock: SimClock,
    observations: Observations,
    knowledge: KnowledgeBases,
    reports: ReportKnowledge,
    namespace: KnowledgeNamespace,
    intents: PendingIntents,
    results: OrderResults,
    applied: AppliedIntents,
    operations: operations::Operations,
    combat: CombatState,
    weapons: iftu::WeaponSystem,
    communications: CommunicationsRuntime,
    transport: transport::Transport,
    reporting_ticks: u64,
    campaign_mode: bool,
    pending_network_events: Vec<NetworkEvent>,
    pending_fragment_events: Vec<NetworkEvent>,
}

impl Simulation {
    /// Snapshot all state that can affect future simulation results.
    #[allow(deprecated)]
    pub fn snapshot(&self) -> SimulationSnapshot {
        let mut query = self.snapshot_query.lock().unwrap();
        let mut ids: Vec<_> = query.iter(&self.world).map(|(entity, _)| entity).collect();
        ids.sort_by_key(|entity| entity.index());
        let platforms = ids
            .into_iter()
            .map(|id| {
                let entity = self.world.entity(id);
                PlatformState {
                    id: *entity.get::<SimEntityId>().unwrap(),
                    name: entity.get::<PlatformName>().unwrap().clone(),
                    sidc: entity.get::<PlatformSidc>().unwrap().clone(),
                    owner: *entity.get::<Ownership>().unwrap(),
                    domain: *entity.get::<DomainKind>().unwrap(),
                    pose: *entity.get::<GeoPose>().unwrap(),
                    velocity: *entity.get::<Velocity>().unwrap(),
                    motion: *entity.get::<GeodesicMotionState>().unwrap(),
                    authority: *entity.get::<AuthorityNode>().unwrap(),
                    combat: entity.get::<operations::CombatState>().unwrap().clone(),
                    sensor: entity.get::<Sensor>().copied(),
                    path: entity.get::<CyclicFlightPathState>().cloned(),
                }
            })
            .collect();
        SimulationSnapshot {
            schema_version: 1,
            platforms,
            clock: *self.world.resource::<SimClock>(),
            observations: self.world.resource::<Observations>().clone(),
            knowledge: self.world.resource::<KnowledgeBases>().clone(),
            reports: self.world.resource::<ReportKnowledge>().clone(),
            namespace: self.world.resource::<KnowledgeNamespace>().clone(),
            intents: self.world.resource::<PendingIntents>().clone(),
            results: self.world.resource::<OrderResults>().clone(),
            applied: self.world.resource::<AppliedIntents>().clone(),
            operations: self.world.resource::<operations::Operations>().clone(),
            combat: self.world.resource::<CombatState>().clone(),
            weapons: self.world.resource::<iftu::WeaponSystem>().clone(),
            communications: self.communications.clone(),
            transport: self.transport.clone(),
            reporting_ticks: self.reporting_ticks,
            campaign_mode: self.campaign_mode,
            pending_network_events: self.pending_network_events.clone(),
            pending_fragment_events: self.pending_fragment_events.clone(),
        }
    }

    /// Restore a trusted, schema-compatible snapshot without advancing its clocks.
    pub fn restore(snapshot: SimulationSnapshot) -> Result<Self, String> {
        if snapshot.schema_version != 1 {
            return Err("incompatible simulation snapshot version".into());
        }
        let mut ids = BTreeSet::new();
        if snapshot
            .platforms
            .iter()
            .any(|platform| !ids.insert(platform.id.0))
        {
            return Err("duplicate platform in snapshot".into());
        }
        let mut world = World::new();
        world.insert_resource(snapshot.clock);
        world.insert_resource(snapshot.observations);
        world.insert_resource(snapshot.knowledge);
        world.insert_resource(snapshot.reports);
        world.insert_resource(snapshot.namespace);
        world.insert_resource(snapshot.intents);
        world.insert_resource(snapshot.results);
        world.insert_resource(snapshot.applied);
        world.insert_resource(snapshot.operations);
        world.insert_resource(snapshot.combat);
        world.insert_resource(snapshot.weapons);
        for platform in snapshot.platforms {
            let mut entity = world.spawn((
                platform.id,
                platform.name,
                platform.sidc,
                platform.owner,
                platform.domain,
                platform.pose,
                platform.velocity,
                platform.motion,
                platform.authority,
                platform.combat,
            ));
            if let Some(sensor) = platform.sensor {
                entity.insert(sensor);
            }
            if let Some(path) = platform.path {
                entity.insert(path);
            }
        }
        let snapshot_query = world.query::<(Entity, &SimEntityId)>();
        let mut schedule = Schedule::default();
        schedule.add_systems((
            advance_clock,
            apply_orders.after(advance_clock),
            operations::advance_operations.after(apply_orders),
            move_platforms.after(operations::advance_operations),
            combat::resolve_impacts.after(move_platforms),
            detect_contacts.after(combat::resolve_impacts),
            deliver_reports.after(detect_contacts),
        ));
        Ok(Self {
            #[cfg(test)]
            compact_network_history: true,
            world,
            snapshot_query: std::sync::Mutex::new(snapshot_query),
            schedule,
            communications: snapshot.communications,
            transport: snapshot.transport,
            reporting_ticks: snapshot.reporting_ticks,
            campaign_mode: snapshot.campaign_mode,
            pending_network_events: snapshot.pending_network_events,
            pending_fragment_events: snapshot.pending_fragment_events,
            last_step_timings: performance::StepTimings::default(),
        })
    }
}

pub(crate) mod network {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        network: &NetworkSimulator,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        network.snapshot().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<NetworkSimulator, D::Error> {
        NetworkSimulator::restore(c3mesh::SimulatorSnapshot::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}

/// Encode compound-key maps as ordered entries; reject duplicate keys on restore.
pub mod entries {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;
    pub fn serialize<K: Serialize, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        map.iter().collect::<Vec<_>>().serialize(serializer)
    }
    pub fn deserialize<
        'de,
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    >(
        deserializer: D,
    ) -> Result<BTreeMap<K, V>, D::Error> {
        let entries = Vec::<(K, V)>::deserialize(deserializer)?;
        let count = entries.len();
        let map: BTreeMap<_, _> = entries.into_iter().collect();
        if count != map.len() {
            return Err(serde::de::Error::custom("duplicate snapshot map key"));
        }
        Ok(map)
    }
}
