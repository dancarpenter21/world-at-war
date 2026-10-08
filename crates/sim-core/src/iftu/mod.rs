//! Simulation-only weapon updates. Every remote observation and update traverses c3mesh.
pub mod catalog;
use super::*;
use c3mesh::{
    ChannelConfig, ChannelOptions, DeviceConfig, DistanceChannel, MobilityModel, Position3D,
    QueueConfig, RadioChannel,
};
use catalog::{Catalog, Seeker, WeaponProfile};
const NS: u64 = 1_000_000_000;
const SUBSTEP: u64 = 100_000_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub equipment: Vec<Equipment>,
    pub loadouts: Vec<Loadout>,
    pub subscriptions: Vec<Subscription>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Equipment {
    pub unit_id: Uuid,
    pub platform_fit: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loadout {
    pub unit_id: Uuid,
    pub weapon_id: String,
    pub provider_id: Uuid,
    pub satellite_relay_id: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub source_id: Uuid,
    pub provider_id: Uuid,
    pub interval_ticks: u64,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Update { track_id: Uuid },
    Retarget { track_id: Uuid },
    AssignProvider { provider_id: Uuid },
    Subscribe { source_id: Uuid, provider_id: Uuid },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeaponTargetUpdate {
    pub weapon_id: Uuid,
    pub provider_id: Uuid,
    pub assignment_revision: u64,
    pub sequence: u64,
    pub source_id: Uuid,
    pub source_track_id: Uuid,
    pub track: Track,
    pub expires_at_ns: u64,
    pub retarget: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Payload {
    Report {
        source: Uuid,
        track: Track,
    },
    Update(WeaponTargetUpdate),
    Assignment {
        weapon: Uuid,
        provider: Uuid,
        revision: u64,
    },
    Control {
        launcher: Uuid,
        weapon: Uuid,
        command: Command,
    },
    Status {
        weapon: Uuid,
        provider: Uuid,
        position: GeoPose,
        phase: String,
        accepted_sequence: u64,
        observed_at_ns: u64,
    },
}
impl Payload {
    fn name(&self) -> &'static str {
        match self {
            Self::Report { .. } => "track_report",
            Self::Update(_) => "weapon_target_update",
            Self::Assignment { .. } => "provider_assignment",
            Self::Control { .. } => "iftu_command",
            Self::Status { .. } => "weapon_status",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTrace {
    pub id: u64,
    pub at_ns: u64,
    pub from: Uuid,
    pub to: Uuid,
    pub kind: String,
    pub state: String,
    pub detail: String,
    pub weapon_id: Option<Uuid>,
    pub fields: serde_json::Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeaponView {
    pub id: Uuid,
    pub launcher_id: Uuid,
    pub weapon_name: String,
    pub provider_id: Uuid,
    pub position: GeoPose,
    pub observed_at_ns: u64,
    pub phase: String,
    pub update_confirmation: String,
    pub retarget_capable: bool,
    pub handoff_capable: bool,
    pub compatible_providers: Vec<Uuid>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Projection {
    pub weapons: Vec<WeaponView>,
    pub messages: Vec<MessageTrace>,
}
struct Flight {
    id: Uuid,
    launcher: Uuid,
    side: Side,
    profile: WeaponProfile,
    provider: Uuid,
    revision: u64,
    initial_provider: Uuid,
    sender_provider: Uuid,
    sender_revision: u64,
    sequence: u64,
    accepted_sequence: u64,
    source: Uuid,
    source_track: Uuid,
    estimate: Track,
    position: GeoPose,
    launch_position: GeoPose,
    launched_ns: u64,
    last_update_tick: u64,
    last_observed_tick: u64,
    phase: String,
    links: BTreeMap<Uuid, Vec<CommunicationLinkDefinition>>,
    return_links: BTreeMap<Uuid, Vec<CommunicationLinkDefinition>>,
    device_ids: Vec<DeviceId>,
    status: BTreeMap<Uuid, (GeoPose, String, u64, u64, Uuid)>,
}
#[derive(Clone)]
pub(super) struct Launch {
    pub launcher: Uuid,
    pub position: GeoPose,
    pub side: Side,
    pub track: Track,
}
struct Datagram {
    id: u64,
    from: Uuid,
    to: Uuid,
    route: Vec<CommunicationLinkDefinition>,
    hop: usize,
    payload: Payload,
    expires: u64,
    priority: u8,
}
#[derive(Resource)]
pub(super) struct WeaponSystem {
    catalog: Catalog,
    config: Configuration,
    flights: BTreeMap<Uuid, Flight>,
    packets: BTreeMap<u64, Datagram>,
    trace: VecDeque<MessageTrace>,
    pub(super) designations: BTreeMap<Uuid, Track>,
    pub(super) provenance: BTreeMap<(Uuid, Uuid), (Uuid, Uuid)>,
    sequence: u64,
    pub launches: Vec<Launch>,
    pub commands: Vec<(Uuid, Uuid, Command)>,
    last_report_tick: BTreeMap<(Uuid, Uuid), u64>,
    clock_ns: u64,
    pub(super) previous_positions: BTreeMap<Uuid, GeoPose>,
}
impl Default for WeaponSystem {
    fn default() -> Self {
        Self {
            catalog: Catalog::bundled().expect("bundled weapon catalog"),
            config: Configuration::default(),
            flights: BTreeMap::new(),
            packets: BTreeMap::new(),
            trace: VecDeque::new(),
            designations: BTreeMap::new(),
            provenance: BTreeMap::new(),
            sequence: 0,
            launches: vec![],
            commands: vec![],
            last_report_tick: BTreeMap::new(),
            clock_ns: 0,
            previous_positions: BTreeMap::new(),
        }
    }
}
impl WeaponSystem {
    pub fn enabled(&self, unit: Uuid) -> bool {
        self.config.loadouts.iter().any(|l| l.unit_id == unit)
    }
    pub fn designate(&mut self, id: Uuid, launcher: Uuid, terminal: Uuid, track: Track) {
        let origin = self
            .provenance
            .get(&(terminal, track.track_id))
            .copied()
            .unwrap_or((terminal, track.track_id));
        self.provenance.insert((launcher, track.track_id), origin);
        self.designations.insert(id, track);
    }
    pub fn validate_command(
        &self,
        launcher: Uuid,
        weapon: Uuid,
        command: &Command,
    ) -> Result<(), String> {
        let f = self.flights.get(&weapon).ok_or("Unknown weapon")?;
        if f.launcher != launcher || f.phase == "impact" || f.phase == "expired" {
            return Err("Weapon is not active under this launcher".into());
        }
        match command {
            Command::Retarget { .. } if !f.profile.retarget => Err("Weapon cannot retarget".into()),
            Command::AssignProvider { provider_id }
                if !f.profile.handoff || !f.links.contains_key(provider_id) =>
            {
                Err("Provider handoff unsupported or incompatible".into())
            }
            Command::Subscribe { provider_id, .. } if !f.links.contains_key(provider_id) => {
                Err("Incompatible provider".into())
            }
            _ => Ok(()),
        }
    }
    fn trace(
        &mut self,
        id: u64,
        from: Uuid,
        to: Uuid,
        payload: &Payload,
        state: &str,
        detail: impl Into<String>,
    ) {
        let weapon_id = match payload {
            Payload::Update(u) => Some(u.weapon_id),
            Payload::Assignment { weapon, .. }
            | Payload::Control { weapon, .. }
            | Payload::Status { weapon, .. } => Some(*weapon),
            _ => None,
        };
        self.trace.push_back(MessageTrace {
            id,
            at_ns: self.clock_ns,
            from,
            to,
            kind: payload.name().into(),
            state: state.into(),
            detail: detail.into(),
            weapon_id,
            fields: serde_json::to_value(payload).expect("finite message"),
        });
        while self.trace.len() > 2048 {
            self.trace.pop_front();
        }
    }
    fn fit(&self, unit: Uuid) -> Option<&catalog::PlatformFit> {
        let e = self.config.equipment.iter().find(|e| e.unit_id == unit)?;
        self.catalog
            .fits
            .iter()
            .find(|f| f.platform == e.platform_fit)
    }
    fn compatible(&self, unit: Uuid, profile: &WeaponProfile) -> Option<String> {
        self.fit(unit)?
            .transmitter_links
            .iter()
            .find(|l| profile.receiver_links.contains(l))
            .cloned()
    }
    pub(super) fn truth_weapons(&self) -> Vec<crate::truth::TruthWeapon> {
        self.flights
            .values()
            .filter(|f| f.phase != "impact" && f.phase != "expired")
            .map(|f| crate::truth::TruthWeapon {
                id: f.id.to_string(),
                kind: "iftu".into(),
                side: Some(f.side),
                launcher_id: Some(f.launcher),
                position: Some(f.position),
                aim_point: None,
                impact_tick: None,
                phase: f.phase.clone(),
                provider_id: Some(f.provider),
            })
            .collect()
    }
    pub(super) fn truth_links(&self) -> Vec<CommunicationLinkDefinition> {
        self.flights
            .values()
            .filter(|f| f.phase != "impact" && f.phase != "expired")
            .flat_map(|f| f.links.values().chain(f.return_links.values()))
            .flatten()
            .cloned()
            .collect()
    }

    pub fn projection(&self, owner: Uuid) -> Projection {
        Projection {
            weapons: self
                .flights
                .values()
                .filter(|f| f.launcher == owner)
                .map(|f| {
                    let (position, phase, observed, confirmation) =
                        if let Some((p, phase, at, seq, _)) = f.status.get(&owner) {
                            (*p, phase.clone(), *at, format!("Update {seq} confirmed"))
                        } else {
                            (
                                f.launch_position,
                                "flight unconfirmed".into(),
                                f.launched_ns,
                                "Unconfirmed; no received weapon status".into(),
                            )
                        };
                    WeaponView {
                        id: f.id,
                        launcher_id: f.launcher,
                        weapon_name: f.profile.name.clone(),
                        provider_id: f.status.get(&owner).map_or(f.initial_provider, |s| s.4),
                        position,
                        observed_at_ns: observed,
                        phase,
                        update_confirmation: confirmation,
                        retarget_capable: f.profile.retarget,
                        handoff_capable: f.profile.handoff,
                        compatible_providers: f.links.keys().copied().collect(),
                    }
                })
                .collect(),
            messages: self
                .trace
                .iter()
                .filter(|m| {
                    (m.to == owner
                        && matches!(m.state.as_str(), "delivered" | "accepted" | "rejected"))
                        || (m.from == owner && matches!(m.state.as_str(), "queued" | "in_transit"))
                })
                .cloned()
                .collect(),
        }
    }
}
fn ecef(p: GeoPose) -> Position3D {
    let r = geodesy::EARTH_RADIUS_M + p.altitude_m;
    let lat = p.latitude_deg.to_radians();
    let lon = p.longitude_deg.to_radians();
    Position3D::new(
        r * lat.cos() * lon.cos(),
        r * lat.cos() * lon.sin(),
        r * lat.sin(),
    )
}
fn distance(a: GeoPose, b: GeoPose) -> f64 {
    geodesy::slant_distance_m(a, b)
}
fn towards(a: GeoPose, b: GeoPose, meters: f64) -> GeoPose {
    let d = distance(a, b);
    if d <= meters {
        return b;
    }
    let fraction = meters / d;
    let lat1 = a.latitude_deg.to_radians();
    let lat2 = b.latitude_deg.to_radians();
    let dl = (b.longitude_deg - a.longitude_deg).to_radians();
    let bearing =
        (dl.sin() * lat2.cos()).atan2(lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dl.cos());
    let ground = geodesy::surface_distance_m(a, b) * fraction;
    geodesy::advance_position(
        a,
        ground * bearing.cos(),
        ground * bearing.sin(),
        (b.altitude_m - a.altitude_m) * fraction,
    )
    .unwrap_or(a)
}
impl Simulation {
    pub fn configure_iftu(&mut self, config: Configuration) -> Result<(), String> {
        if self.tick() != 0 {
            return Err("weapon configuration must be installed before start".into());
        }
        let catalog = Catalog::bundled()?;
        let mut q = self.world.query::<(&SimEntityId, &Ownership)>();
        let sides: BTreeMap<_, _> = q.iter(&self.world).map(|(id, s)| (id.0, s.0)).collect();
        let mut ids = BTreeSet::new();
        for e in &config.equipment {
            if !sides.contains_key(&e.unit_id)
                || !ids.insert(e.unit_id)
                || !catalog.fits.iter().any(|f| f.platform == e.platform_fit)
            {
                return Err("invalid or duplicate weapon equipment assignment".into());
            }
        }
        let mut system = WeaponSystem {
            catalog,
            config,
            ..Default::default()
        };
        ids.clear();
        for l in &system.config.loadouts {
            let w = system
                .catalog
                .weapon(&l.weapon_id)
                .ok_or("unknown weapon")?;
            if !ids.insert(l.unit_id)
                || system
                    .fit(l.unit_id)
                    .is_none_or(|f| !f.weapons.contains(&l.weapon_id))
                || sides.get(&l.unit_id) != sides.get(&l.provider_id)
                || !sides.contains_key(&l.unit_id)
            {
                return Err("invalid loadout or provider side".into());
            }
            if !w.receiver_links.is_empty() && system.compatible(l.provider_id, w).is_none() {
                return Err("provider lacks compatible IFTU transmitter".into());
            }
            if w.receiver_links
                .iter()
                .any(|id| system.catalog.link(id).unwrap().satellite)
                && l.satellite_relay_id
                    .is_none_or(|id| !sides.contains_key(&id))
            {
                return Err("satellite weapon link requires an explicit relay entity".into());
            }
        }
        for s in &system.config.subscriptions {
            if s.interval_ticks == 0
                || !sides.contains_key(&s.source_id)
                || sides.get(&s.source_id) != sides.get(&s.provider_id)
            {
                return Err("invalid observation subscription".into());
            }
        }
        system.clock_ns = self.communications.simulator.now().as_nanos();
        self.world
            .resource_mut::<operations::Operations>()
            .iftu_units = system.config.loadouts.iter().map(|l| l.unit_id).collect();
        self.world.insert_resource(system);
        Ok(())
    }
    pub fn iftu_projection(&self, owner: Uuid) -> Projection {
        self.world.resource::<WeaponSystem>().projection(owner)
    }
    fn weapon_positions(&mut self) -> BTreeMap<Uuid, (GeoPose, Side, bool)> {
        let mut q = self
            .world
            .query::<(&SimEntityId, &GeoPose, &Ownership, &operations::CombatState)>();
        q.iter(&self.world)
            .map(|(id, p, s, c)| (id.0, (*p, s.0, c.destroyed)))
            .collect()
    }
    fn weapon_positions_at(
        &mut self,
        s: &WeaponSystem,
        at: u64,
    ) -> BTreeMap<Uuid, (GeoPose, Side, bool)> {
        let mut positions = self.weapon_positions();
        let start = self.tick().saturating_sub(1) * NS;
        let fraction = (at.saturating_sub(start) as f64 / NS as f64).clamp(0.0, 1.0);
        for (id, (position, _, _)) in &mut positions {
            if let Some(previous) = s.previous_positions.get(id) {
                *position = towards(
                    *previous,
                    *position,
                    distance(*previous, *position) * fraction,
                );
            }
        }
        positions
    }
    pub(super) fn capture_iftu_positions(&mut self) {
        if self
            .world
            .resource::<WeaponSystem>()
            .config
            .loadouts
            .is_empty()
        {
            return;
        }
        let positions = self
            .weapon_positions()
            .into_iter()
            .map(|(id, (p, _, _))| (id, p))
            .collect();
        self.world.resource_mut::<WeaponSystem>().previous_positions = positions;
    }
    fn iftu_route(&self, from: Uuid, to: Uuid) -> Option<Vec<CommunicationLinkDefinition>> {
        if from == to {
            return Some(vec![]);
        }
        let mut queue = VecDeque::from([(from, vec![])]);
        let mut seen = BTreeSet::from([from]);
        while let Some((node, path)) = queue.pop_front() {
            for l in self
                .communications
                .links
                .iter()
                .filter(|l| l.from_entity_id == node)
            {
                if seen.insert(l.to_entity_id) {
                    let mut p = path.clone();
                    p.push(l.clone());
                    if l.to_entity_id == to {
                        return Some(p);
                    }
                    queue.push_back((l.to_entity_id, p));
                }
            }
        }
        None
    }
    fn send_iftu_datagram(
        &mut self,
        s: &mut WeaponSystem,
        from: Uuid,
        to: Uuid,
        payload: Payload,
        route: Option<Vec<CommunicationLinkDefinition>>,
    ) {
        s.sequence += 1;
        let id = s.sequence;
        let expires = s.clock_ns + 15 * NS;
        let route = route.or_else(|| self.iftu_route(from, to));
        if let Some(route) = route {
            let d = Datagram {
                id,
                from,
                to,
                route,
                hop: 0,
                priority: if matches!(payload, Payload::Report { .. }) {
                    120
                } else {
                    240
                },
                payload,
                expires,
            };
            if d.route.is_empty() {
                self.deliver_iftu(s, d);
            } else {
                self.queue_iftu_hop(s, d);
            }
        } else {
            s.trace(id, from, to, &payload, "dropped", "No configured route");
        }
    }
    fn queue_iftu_hop(&mut self, s: &mut WeaponSystem, d: Datagram) {
        let link = &d.route[d.hop];
        let positions = self.weapon_positions();
        if [link.from_entity_id, link.to_entity_id]
            .iter()
            .any(|id| positions.get(id).is_some_and(|p| p.2))
        {
            s.trace(
                d.id,
                d.from,
                d.to,
                &d.payload,
                "dropped",
                "Endpoint destroyed",
            );
            return;
        }
        if s.clock_ns >= d.expires || d.hop >= 32 {
            s.trace(
                d.id,
                d.from,
                d.to,
                &d.payload,
                "expired",
                "Expiry or hop limit",
            );
            return;
        }
        let bytes = serde_json::to_vec(&d.payload).expect("finite IFTU payload");
        let result = self
            .communications
            .simulator
            .schedule_send_with_hop_limit_and_metadata(
                NetworkTime::from_nanos(s.clock_ns),
                link.source_device_id.clone(),
                link.destination_device_id.clone(),
                bytes,
                32 - d.hop as u16,
                PacketMetadata {
                    priority: d.priority,
                    traffic_class: d.priority / 64,
                    flow_id: d.from.as_u128() as u64,
                    expires_at: Some(NetworkTime::from_nanos(d.expires)),
                },
            );
        match result {
            Ok(packet) => {
                s.trace(
                    d.id,
                    link.from_entity_id,
                    link.to_entity_id,
                    &d.payload,
                    "queued",
                    format!("Hop {} of {}", d.hop + 1, d.route.len()),
                );
                s.packets.insert(packet.get(), d);
            }
            Err(e) => s.trace(d.id, d.from, d.to, &d.payload, "dropped", e.to_string()),
        }
    }
    fn process_iftu_event(
        &mut self,
        s: &mut WeaponSystem,
        event: NetworkEvent,
    ) -> Option<NetworkEvent> {
        let (id, terminal, reason) = match &event {
            NetworkEvent::TransmissionStarted { packet, .. } => (packet.id().get(), false, None),
            NetworkEvent::PacketDelivered { packet, .. } => (packet.id().get(), true, None),
            NetworkEvent::PacketDropped { packet, reason, .. } => {
                (packet.id().get(), true, Some(format!("{reason:?}")))
            }
            NetworkEvent::DataReceived { packet, .. } => {
                if s.packets.contains_key(&packet.id().get()) {
                    return None;
                } else {
                    return Some(event);
                }
            }
        };
        let Some(d) = s.packets.get(&id) else {
            return Some(event);
        };
        if !terminal {
            let (id, from, to, payload) = (
                d.id,
                d.route[d.hop].from_entity_id,
                d.route[d.hop].to_entity_id,
                d.payload.clone(),
            );
            s.trace(
                id,
                from,
                to,
                &payload,
                "in_transit",
                "Serialization started",
            );
            return None;
        }
        let mut d = s.packets.remove(&id).unwrap();
        if let Some(reason) = reason {
            s.trace(d.id, d.from, d.to, &d.payload, "dropped", reason);
            return None;
        }
        s.trace(
            d.id,
            d.route[d.hop].from_entity_id,
            d.route[d.hop].to_entity_id,
            &d.payload,
            "delivered",
            "Packet received",
        );
        d.hop += 1;
        if d.hop < d.route.len() {
            self.queue_iftu_hop(s, d)
        } else {
            self.deliver_iftu(s, d)
        }
        None
    }
    fn deliver_iftu(&mut self, s: &mut WeaponSystem, d: Datagram) {
        if s.clock_ns >= d.expires {
            s.trace(
                d.id,
                d.from,
                d.to,
                &d.payload,
                "expired",
                "Expired on receipt",
            );
            return;
        }
        let positions = self.weapon_positions();
        if positions.get(&d.to).is_some_and(|(_, _, dead)| *dead) {
            s.trace(
                d.id,
                d.from,
                d.to,
                &d.payload,
                "rejected",
                "Recipient destroyed",
            );
            return;
        }
        let mut result = Ok(());
        match &d.payload {
            Payload::Report { source, track } => {
                if self.receive_track_report(d.to, track.clone()) {
                    let local = scoped_track_id(
                        self.world.resource::<KnowledgeNamespace>().0,
                        d.to,
                        track.track_id,
                    );
                    s.provenance
                        .insert((d.to, local), (*source, track.track_id));
                }
            }
            Payload::Update(u) => {
                result = (|| {
                    let f = s.flights.get_mut(&u.weapon_id).ok_or("Unknown weapon")?;
                    if f.phase == "impact" || f.phase == "expired" {
                        return Err("Weapon already terminal");
                    }
                    if d.to != f.id
                        || u.track.target_side.is_none_or(|side| side == f.side)
                        || !u.track.identity_confidence.is_finite()
                        || u.track.identity_confidence < 0.8
                    {
                        return Err("Invalid receiver or target identification");
                    }
                    if u.provider_id != f.provider
                        || u.assignment_revision != f.revision
                        || d.from != f.provider
                    {
                        return Err("Obsolete or unauthorized provider");
                    }
                    if u.sequence <= f.accepted_sequence || u.expires_at_ns <= s.clock_ns {
                        return Err("Duplicate or expired update");
                    }
                    if u.track.observed_tick > s.clock_ns / NS
                        || s.clock_ns / NS - u.track.observed_tick
                            > f.profile.max_observation_age_ticks
                        || !geo_pose_is_finite(u.track.position)
                    {
                        return Err("Invalid or stale observation");
                    }
                    if !u.retarget
                        && (u.source_id != f.source
                            || u.source_track_id != f.source_track
                            || u.track.observed_tick <= f.estimate.observed_tick)
                    {
                        return Err("Unbound or out-of-order observation");
                    }
                    if u.retarget && !f.profile.retarget {
                        return Err("Weapon cannot retarget");
                    }
                    f.source = u.source_id;
                    f.source_track = u.source_track_id;
                    f.estimate = u.track.clone();
                    f.accepted_sequence = u.sequence;
                    Ok(())
                })();
            }
            Payload::Assignment {
                weapon,
                provider,
                revision,
            } => {
                result = (|| {
                    let f = s.flights.get_mut(weapon).ok_or("Unknown weapon")?;
                    if !f.profile.handoff
                        || !f.links.contains_key(provider)
                        || *revision <= f.revision
                        || d.from != f.launcher
                    {
                        return Err("Invalid provider assignment");
                    };
                    f.provider = *provider;
                    f.revision = *revision;
                    Ok(())
                })();
            }
            Payload::Control {
                launcher,
                weapon,
                command,
            } => {
                result = self
                    .execute_iftu_command(s, *launcher, *weapon, command.clone(), d.to)
                    .map_err(|_| "IFTU command rejected");
            }
            Payload::Status {
                weapon,
                provider,
                position,
                phase,
                accepted_sequence,
                observed_at_ns,
            } => {
                if let Some(f) = s.flights.get_mut(weapon) {
                    if (d.from == f.id || d.from == f.provider)
                        && f.status
                            .get(&d.to)
                            .is_none_or(|(_, _, at, _, _)| at < observed_at_ns)
                    {
                        f.status.insert(
                            d.to,
                            (
                                *position,
                                phase.clone(),
                                *observed_at_ns,
                                *accepted_sequence,
                                *provider,
                            ),
                        );
                    }
                }
            }
        }
        if let Payload::Status { weapon, .. } = &d.payload {
            if let Some(f) = s.flights.get(weapon) {
                if d.to == f.provider && f.launcher != f.provider {
                    let launcher = f.launcher;
                    self.send_iftu_datagram(s, d.to, launcher, d.payload.clone(), None);
                }
            }
        }
        s.trace(
            d.id,
            d.from,
            d.to,
            &d.payload,
            if result.is_ok() {
                "accepted"
            } else {
                "rejected"
            },
            result.err().unwrap_or("Application received message"),
        );
    }
    fn execute_iftu_command(
        &mut self,
        s: &mut WeaponSystem,
        launcher: Uuid,
        weapon: Uuid,
        command: Command,
        recipient: Uuid,
    ) -> Result<(), String> {
        let positions = self.weapon_positions();
        let f = s.flights.get(&weapon).ok_or("Unknown weapon")?;
        if f.launcher != launcher || f.phase == "impact" || f.phase == "expired" {
            return Err("Not an active weapon of this launcher".into());
        }
        match command {
            Command::Subscribe {
                source_id,
                provider_id,
            } => {
                if recipient != source_id
                    || positions.get(&source_id).map(|v| v.1) != Some(f.side)
                    || !f.links.contains_key(&provider_id)
                {
                    return Err("Invalid observation subscription".into());
                }
                if !s
                    .config
                    .subscriptions
                    .iter()
                    .any(|v| v.source_id == source_id && v.provider_id == provider_id)
                {
                    s.config.subscriptions.push(Subscription {
                        source_id,
                        provider_id,
                        interval_ticks: 2,
                    });
                }
            }
            Command::AssignProvider { provider_id } => {
                if !f.profile.handoff {
                    return Err("Provider handoff unsupported".into());
                }
                let route = f
                    .links
                    .get(&provider_id)
                    .ok_or("Incompatible provider")?
                    .clone();
                if recipient != provider_id {
                    return Err("Handoff command reached the wrong provider".into());
                }
                let revision = f.sender_revision + 1;
                let f = s.flights.get_mut(&weapon).unwrap();
                f.sender_provider = provider_id;
                f.sender_revision = revision;
                self.send_iftu_datagram(
                    s,
                    launcher,
                    weapon,
                    Payload::Assignment {
                        weapon,
                        provider: provider_id,
                        revision,
                    },
                    Some(route),
                );
            }
            Command::Update { track_id } | Command::Retarget { track_id } => {
                if recipient != f.sender_provider {
                    return Err("Command must reach current provider".into());
                }
                let retarget = matches!(command, Command::Retarget { .. });
                let track = self
                    .world
                    .resource::<KnowledgeBases>()
                    .0
                    .get(&recipient)
                    .and_then(|ts| ts.iter().find(|t| t.track_id == track_id))
                    .cloned()
                    .ok_or("Track unknown to provider")?;
                self.make_iftu(s, weapon, track, retarget)?;
            }
        }
        Ok(())
    }
    fn make_iftu(
        &mut self,
        s: &mut WeaponSystem,
        weapon: Uuid,
        track: Track,
        retarget: bool,
    ) -> Result<(), String> {
        let f = s.flights.get_mut(&weapon).ok_or("Unknown weapon")?;
        if s.clock_ns / NS < track.observed_tick
            || s.clock_ns / NS - track.observed_tick > f.profile.max_observation_age_ticks
            || track.identity_confidence < 0.8
            || track.target_side == Some(f.side)
            || track.target_side.is_none()
        {
            return Err("Observation does not meet engagement constraints".into());
        }
        let (source, source_track) = s
            .provenance
            .get(&(f.sender_provider, track.track_id))
            .copied()
            .unwrap_or((f.sender_provider, track.track_id));
        if !retarget && (source != f.source || source_track != f.source_track) {
            return Err(
                "Observation is not bound to this engagement; use authorized retargeting".into(),
            );
        }
        if retarget && !f.profile.retarget {
            return Err("Weapon does not support retargeting".into());
        }
        let route = f
            .links
            .get(&f.sender_provider)
            .ok_or("No compatible weapon link")?
            .clone();
        f.sequence += 1;
        f.last_observed_tick = track.observed_tick;
        let update = WeaponTargetUpdate {
            weapon_id: weapon,
            provider_id: f.sender_provider,
            assignment_revision: f.sender_revision,
            sequence: f.sequence,
            source_id: source,
            source_track_id: source_track,
            track,
            expires_at_ns: s.clock_ns + f.profile.max_observation_age_ticks * NS,
            retarget,
        };
        let provider = f.sender_provider;
        self.send_iftu_datagram(s, provider, weapon, Payload::Update(update), Some(route));
        Ok(())
    }
    fn prepare_iftu(&mut self, s: &mut WeaponSystem) {
        let positions = self.weapon_positions();
        s.launches.extend(std::mem::take(
            &mut self.world.resource_mut::<operations::Operations>().launches,
        ));
        for launch in std::mem::take(&mut s.launches) {
            if let Err(error) = self.spawn_iftu_weapon(s, launch) {
                s.sequence += 1;
                s.trace.push_back(MessageTrace {
                    id: s.sequence,
                    at_ns: s.clock_ns,
                    from: Uuid::nil(),
                    to: Uuid::nil(),
                    kind: "launch".into(),
                    state: "rejected".into(),
                    detail: error,
                    weapon_id: None,
                    fields: serde_json::Value::Null,
                });
            }
        }
        for (launcher, weapon, command) in std::mem::take(&mut s.commands) {
            let Some(f) = s.flights.get(&weapon) else {
                continue;
            };
            let recipient = match command {
                Command::Subscribe { source_id, .. } => source_id,
                Command::AssignProvider { provider_id } => provider_id,
                _ => f.sender_provider,
            };
            self.send_iftu_datagram(
                s,
                launcher,
                recipient,
                Payload::Control {
                    launcher,
                    weapon,
                    command,
                },
                None,
            );
        }
        for sub in s.config.subscriptions.clone() {
            if positions.get(&sub.source_id).is_none_or(|p| p.2)
                || positions.get(&sub.provider_id).is_none_or(|p| p.2)
            {
                continue;
            }
            let key = (sub.source_id, sub.provider_id);
            let tick = s.clock_ns / NS;
            if s.last_report_tick
                .get(&key)
                .is_some_and(|t| tick.saturating_sub(*t) < sub.interval_ticks)
            {
                continue;
            }
            s.last_report_tick.insert(key, tick);
            for track in self.current_sensor_tracks(sub.source_id) {
                self.send_iftu_datagram(
                    s,
                    sub.source_id,
                    sub.provider_id,
                    Payload::Report {
                        source: sub.source_id,
                        track,
                    },
                    None,
                );
            }
        }
        let ids: Vec<_> = s.flights.keys().copied().collect();
        for id in ids {
            let f = s.flights.get_mut(&id).unwrap();
            let tick = s.clock_ns / NS;
            if f.phase == "impact"
                || f.phase == "expired"
                || f.profile.receiver_links.is_empty()
                || positions.get(&f.sender_provider).is_none_or(|p| p.2)
                || tick.saturating_sub(f.last_update_tick) < f.profile.update_interval_ticks
            {
                continue;
            }
            f.last_update_tick = tick;
            let provider = f.sender_provider;
            let source = f.source;
            let source_track = f.source_track;
            let observed = f.last_observed_tick;
            let track = self
                .world
                .resource::<KnowledgeBases>()
                .0
                .get(&provider)
                .and_then(|tracks| {
                    tracks
                        .iter()
                        .filter(|t| t.observed_tick > observed)
                        .find(|t| {
                            s.provenance
                                .get(&(provider, t.track_id))
                                .copied()
                                .unwrap_or((provider, t.track_id))
                                == (source, source_track)
                        })
                })
                .cloned();
            if let Some(track) = track {
                let _ = self.make_iftu(s, id, track, false);
            }
        }
    }
    fn spawn_iftu_weapon(&mut self, s: &mut WeaponSystem, launch: Launch) -> Result<(), String> {
        let load = s
            .config
            .loadouts
            .iter()
            .find(|l| l.unit_id == launch.launcher)
            .ok_or("No weapon loadout")?
            .clone();
        let profile = s.catalog.weapon(&load.weapon_id).unwrap().clone();
        s.sequence += 1;
        let id = Uuid::new_v5(
            &self.world.resource::<KnowledgeNamespace>().0,
            format!("weapon:{}:{}", launch.launcher, s.sequence).as_bytes(),
        );
        let positions = self.weapon_positions();
        let mut config = NetworkConfig::default();
        let mut options = SimulatorOptions::default();
        let mut links = BTreeMap::new();
        let mut returns = BTreeMap::new();
        let candidates: Vec<_> = s
            .config
            .equipment
            .iter()
            .filter(|e| {
                positions
                    .get(&e.unit_id)
                    .is_some_and(|p| p.1 == launch.side && !p.2)
            })
            .filter_map(|e| s.compatible(e.unit_id, &profile).map(|l| (e.unit_id, l)))
            .collect();
        for (provider, link_id) in candidates {
            let link = s.catalog.link(&link_id).unwrap();
            let nodes = if link.satellite {
                vec![
                    provider,
                    load.satellite_relay_id.ok_or("No satellite relay")?,
                    id,
                ]
            } else {
                vec![provider, id]
            };
            for reverse in [false, true] {
                if reverse && !profile.telemetry {
                    continue;
                }
                let nodes: Vec<_> = if reverse {
                    nodes.iter().rev().copied().collect()
                } else {
                    nodes.clone()
                };
                let mut path = vec![];
                for (hop, pair) in nodes.windows(2).enumerate() {
                    let label = format!("iftu:{id}:{provider}:{reverse}:{hop}");
                    let channel: ChannelId = label.clone().into();
                    let source: DeviceId = format!("{label}:tx").into();
                    let sink: DeviceId = format!("{label}:rx").into();
                    for (device, node, kind) in [
                        (
                            source.clone(),
                            pair[0],
                            DeviceKind::Source {
                                egress: channel.clone(),
                            },
                        ),
                        (sink.clone(), pair[1], DeviceKind::Sink),
                    ] {
                        let position = if node == id {
                            launch.position
                        } else {
                            positions[&node].0
                        };
                        config.devices.push(DeviceConfig {
                            id: device,
                            kind,
                            mobility: MobilityModel::Static {
                                position: ecef(position),
                            },
                            interference: vec![],
                        });
                    }
                    config.channels.push(ChannelConfig {
                        id: channel.clone(),
                        endpoints: [source.clone(), sink.clone()],
                        bit_rate_bps: link.bit_rate_bps,
                        propagation_delay_ns: 100_000,
                        state: CommunicationChannelState::Operational,
                        distance: Some(DistanceChannel {
                            propagation_speed_mps: 299_792_458.0,
                            max_range_m: link.range_m,
                            rate_model: Default::default(),
                        }),
                        radio: Some(RadioChannel {
                            band: link.band,
                            interference_response: Default::default(),
                        }),
                    });
                    options.channels.insert(
                        channel.clone(),
                        ChannelOptions {
                            mtu_bytes: Some(8192),
                            queue: QueueConfig {
                                max_packets: Some(32),
                                max_bytes: Some(65536),
                                discipline: c3mesh::QueueDiscipline::WeightedFair,
                            },
                            shared_medium: Some(format!("iftu:{provider}:{link_id}")),
                            wire_overhead_bytes: 32,
                            ..Default::default()
                        },
                    );
                    path.push(CommunicationLinkDefinition {
                        id: label,
                        from_entity_id: pair[0],
                        to_entity_id: pair[1],
                        source_device_id: source,
                        destination_device_id: sink,
                        channel_id: channel,
                    });
                }
                if reverse {
                    returns.insert(provider, path);
                } else {
                    links.insert(provider, path);
                }
            }
        }
        let device_ids = config.devices.iter().map(|d| d.id.clone()).collect();
        self.communications
            .simulator
            .register_topology(config, options)
            .map_err(|e| e.to_string())?;
        let (source, source_track) = s
            .provenance
            .get(&(launch.launcher, launch.track.track_id))
            .copied()
            .unwrap_or((launch.launcher, launch.track.track_id));
        s.flights.insert(
            id,
            Flight {
                id,
                launcher: launch.launcher,
                side: launch.side,
                profile,
                provider: load.provider_id,
                revision: 1,
                initial_provider: load.provider_id,
                sender_provider: load.provider_id,
                sender_revision: 1,
                sequence: 0,
                accepted_sequence: 0,
                source,
                source_track,
                last_observed_tick: launch.track.observed_tick,
                estimate: launch.track,
                position: launch.position,
                launch_position: launch.position,
                launched_ns: s.clock_ns,
                last_update_tick: s.clock_ns / NS,
                phase: "midcourse".into(),
                links,
                return_links: returns,
                device_ids,
                status: BTreeMap::new(),
            },
        );
        Ok(())
    }
    fn sync_iftu_links(&mut self, s: &WeaponSystem) -> Result<(), c3mesh::SimulationError> {
        let positions = self.weapon_positions_at(s, s.clock_ns);
        for f in s
            .flights
            .values()
            .filter(|f| f.phase != "impact" && f.phase != "expired")
        {
            for path in f.links.values().chain(f.return_links.values()) {
                for link in path {
                    let from = if link.from_entity_id == f.id {
                        Some((f.position, f.side, false))
                    } else {
                        positions.get(&link.from_entity_id).copied()
                    };
                    let to = if link.to_entity_id == f.id {
                        Some((f.position, f.side, false))
                    } else {
                        positions.get(&link.to_entity_id).copied()
                    };
                    let (Some((from, _, from_dead)), Some((to, _, to_dead))) = (from, to) else {
                        continue;
                    };
                    self.communications.simulator.set_device_mobility(
                        link.source_device_id.clone(),
                        MobilityModel::Static {
                            position: ecef(from),
                        },
                    )?;
                    self.communications.simulator.set_device_mobility(
                        link.destination_device_id.clone(),
                        MobilityModel::Static { position: ecef(to) },
                    )?;
                    let mut interference: Vec<_> = self
                        .communications
                        .jamming_regions
                        .iter()
                        .filter(|r| {
                            r.active_at(s.clock_ns / NS)
                                && great_circle_distance_m(to, r.center) <= r.radius_m
                        })
                        .map(|r| ReceiverInterference {
                            band: r.band,
                            jammed: r.jammed,
                        })
                        .collect();
                    if from_dead || to_dead || !geodesy::has_geometric_line_of_sight(from, to) {
                        interference.push(ReceiverInterference {
                            band: FrequencyBand::new(1, u64::MAX),
                            jammed: 1.0,
                        });
                    }
                    self.communications.simulator.set_receiver_interference(
                        link.destination_device_id.clone(),
                        interference,
                    )?;
                }
            }
        }
        Ok(())
    }
    fn move_iftu_weapons(&mut self, s: &mut WeaponSystem, until: u64) {
        let dt = (until - s.clock_ns) as f64 / NS as f64;
        if dt == 0.0 {
            return;
        }
        let positions = self.weapon_positions_at(s, until);
        let mut hits = BTreeSet::new();
        let mut status = vec![];
        let mut retired = vec![];
        for f in s
            .flights
            .values_mut()
            .filter(|f| f.phase != "impact" && f.phase != "expired")
        {
            let mut aim = f.estimate.position;
            if until - f.launched_ns >= (f.profile.lifetime_seconds * NS as f64) as u64 {
                f.phase = "expired".into();
            } else {
                if matches!(f.profile.seeker, Seeker::Local) {
                    // Bounded local observation only; the engagement contains no truth target ID.
                    if let Some((_, (target, _, _))) = positions
                        .iter()
                        .filter(|(_, (p, side, dead))| {
                            *side != f.side
                                && !*dead
                                && distance(f.position, *p) <= f.profile.acquisition_m
                                && distance(f.estimate.position, *p) <= f.profile.acquisition_m
                                && geodesy::has_geometric_line_of_sight(f.position, *p)
                        })
                        .min_by(|a, b| {
                            distance(f.estimate.position, a.1 .0)
                                .total_cmp(&distance(f.estimate.position, b.1 .0))
                                .then(a.0.cmp(b.0))
                        })
                    {
                        aim = *target;
                        f.phase = "terminal".into();
                    }
                }
                f.position = towards(f.position, aim, f.profile.speed_mps * dt);
                if distance(f.position, aim) <= f.profile.effect_radius_m {
                    f.phase = "impact".into();
                    for (id, (p, side, dead)) in &positions {
                        if *side != f.side
                            && !*dead
                            && distance(f.position, *p) <= f.profile.effect_radius_m
                        {
                            hits.insert(*id);
                        }
                    }
                }
            }
            if (s.clock_ns / NS != until / NS || f.phase == "impact" || f.phase == "expired")
                && f.profile.telemetry
            {
                if let Some(route) = f.return_links.get(&f.provider) {
                    status.push((
                        f.id,
                        f.provider,
                        Payload::Status {
                            weapon: f.id,
                            provider: f.provider,
                            position: f.position,
                            phase: f.phase.clone(),
                            accepted_sequence: f.accepted_sequence,
                            observed_at_ns: until,
                        },
                        route.clone(),
                    ));
                }
            }
            if f.phase == "impact" || f.phase == "expired" {
                retired.push(f.device_ids.clone());
            }
        }
        s.clock_ns = until;
        // A terminal weapon cannot transmit after its receiver/transmitter is retired.
        status.retain(|(id, _, _, _)| {
            s.flights
                .get(id)
                .is_some_and(|f| f.phase != "impact" && f.phase != "expired")
        });
        for (from, to, payload, route) in status {
            self.send_iftu_datagram(s, from, to, payload, Some(route));
        }
        for devices in retired {
            let _ = self.communications.simulator.retire_devices(&devices);
        }
        let mut q = self
            .world
            .query::<(&SimEntityId, &mut operations::CombatState, &mut Velocity)>();
        for (id, mut combat, mut velocity) in q.iter_mut(&mut self.world) {
            if hits.contains(&id.0) {
                combat.destroyed = true;
                *velocity = Velocity::default();
            }
        }
    }
    pub(super) fn advance_iftu_network(
        &mut self,
        boundary: NetworkTime,
    ) -> Result<Vec<NetworkEvent>, c3mesh::SimulationError> {
        if self
            .world
            .resource::<WeaponSystem>()
            .config
            .loadouts
            .is_empty()
            && self
                .world
                .resource::<WeaponSystem>()
                .config
                .subscriptions
                .is_empty()
        {
            return self.communications.simulator.advance_to(boundary);
        }
        let mut s = self
            .world
            .remove_resource::<WeaponSystem>()
            .unwrap_or_default();
        let mut external = vec![];
        let result = (|| {
            s.clock_ns = self.communications.simulator.now().as_nanos();
            while s.clock_ns < boundary.as_nanos()
                || self
                    .communications
                    .simulator
                    .next_event_time()
                    .is_some_and(|t| t <= boundary && t.as_nanos() == s.clock_ns)
            {
                let mut next = if s
                    .flights
                    .values()
                    .any(|f| f.phase != "impact" && f.phase != "expired")
                {
                    (s.clock_ns + SUBSTEP).min(boundary.as_nanos())
                } else {
                    boundary.as_nanos()
                };
                if let Some(at) = self.communications.simulator.next_event_time() {
                    next = next.min(at.as_nanos());
                }
                // Advance the network before moving to the same boundary so registration and retirement use that exact virtual time.
                self.sync_iftu_links(&s)?;
                let events = self
                    .communications
                    .simulator
                    .advance_to(NetworkTime::from_nanos(next))?;
                self.move_iftu_weapons(&mut s, next);
                for event in events {
                    if let Some(e) = self.process_iftu_event(&mut s, event) {
                        external.push(e);
                    }
                }
            }
            self.prepare_iftu(&mut s);
            Ok(external)
        })();
        self.world.insert_resource(s);
        result
    }
}

#[cfg(test)]
mod tests;
