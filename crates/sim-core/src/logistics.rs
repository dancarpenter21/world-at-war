//! Optional, deterministic logistics. Real profiles require evidence; fixtures are fictional.
use super::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: String,
    pub locator: String,
    pub retrieved: String,
    pub applicability: String,
    pub license: String,
}
impl Evidence {
    fn validate_for(&self, fictional: bool) -> Result<(), String> {
        self.validate()?;
        if !(fictional
            || self.source.starts_with("https://")
            || self.source.starts_with("licensed:"))
        {
            return Err("real logistics parameters require public or licensed evidence; fictional fallback is unavailable".into());
        }
        Ok(())
    }
    fn validate(&self) -> Result<(), String> {
        if [
            &self.source,
            &self.locator,
            &self.retrieved,
            &self.applicability,
            &self.license,
        ]
        .iter()
        .any(|s| s.trim().is_empty())
        {
            return Err(
                "logistics evidence must include source, locator, date, applicability and license"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurnTable {
    pub altitude_m: Vec<f64>,
    pub airspeed_mps: Vec<f64>,
    pub gross_mass_kg: Vec<f64>,
    /// Altitude-major, speed-middle, mass-minor cells, in kg per game second.
    pub kg_per_second: Vec<f64>,
    pub evidence: Evidence,
}
impl BurnTable {
    fn validate(&self) -> Result<(), String> {
        self.evidence.validate()?;
        let valid_axis = |xs: &[f64]| {
            !xs.is_empty()
                && xs.iter().all(|x| x.is_finite() && *x >= 0.0)
                && xs.windows(2).all(|w| w[0] < w[1])
        };
        if !valid_axis(&self.altitude_m)
            || !valid_axis(&self.airspeed_mps)
            || !valid_axis(&self.gross_mass_kg)
            || self.gross_mass_kg[0] <= 0.0
            || self.kg_per_second.len()
                != self.altitude_m.len() * self.airspeed_mps.len() * self.gross_mass_kg.len()
            || self
                .kg_per_second
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err("invalid fuel performance table".into());
        }
        Ok(())
    }
    pub fn rate(&self, altitude: f64, speed: f64, mass: f64) -> Option<f64> {
        fn bracket(xs: &[f64], x: f64) -> Option<(usize, usize, f64)> {
            if !x.is_finite() || x < *xs.first()? || x > *xs.last()? {
                return None;
            }
            let hi = xs.partition_point(|v| *v < x).min(xs.len() - 1);
            let lo = hi.saturating_sub(1);
            let fraction = if lo == hi {
                0.0
            } else {
                (x - xs[lo]) / (xs[hi] - xs[lo])
            };
            Some((lo, hi, fraction))
        }
        let (al, ah, at) = bracket(&self.altitude_m, altitude)?;
        let (sl, sh, st) = bracket(&self.airspeed_mps, speed)?;
        let (ml, mh, mt) = bracket(&self.gross_mass_kg, mass)?;
        let cell = |a, s, m| {
            self.kg_per_second[(a * self.airspeed_mps.len() + s) * self.gross_mass_kg.len() + m]
        };
        let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
        Some(lerp(
            lerp(
                lerp(cell(al, sl, ml), cell(al, sl, mh), mt),
                lerp(cell(al, sh, ml), cell(al, sh, mh), mt),
                st,
            ),
            lerp(
                lerp(cell(ah, sl, ml), cell(ah, sl, mh), mt),
                lerp(cell(ah, sh, ml), cell(ah, sh, mh), mt),
                st,
            ),
            at,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interface {
    Ground,
    Boom,
    Drogue,
    Underway,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Station {
    pub interface: Interface,
    pub rate_kg_s: f64,
    pub evidence: Evidence,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub min_altitude_m: f64,
    pub max_altitude_m: f64,
    pub min_speed_mps: f64,
    pub max_speed_mps: f64,
    pub max_relative_speed_mps: f64,
    pub connection_distance_m: f64,
    pub connection_seconds: f64,
    pub disconnection_seconds: f64,
    pub max_sea_state: f64,
    pub evidence: Evidence,
}
impl Envelope {
    fn validate(&self) -> Result<(), String> {
        self.evidence.validate()?;
        if [
            self.min_altitude_m,
            self.max_altitude_m,
            self.min_speed_mps,
            self.max_speed_mps,
            self.max_relative_speed_mps,
            self.connection_distance_m,
            self.connection_seconds,
            self.disconnection_seconds,
            self.max_sea_state,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v < 0.0)
            || self.min_altitude_m > self.max_altitude_m
            || self.min_speed_mps > self.max_speed_mps
            || self.connection_distance_m <= 0.0
            || self.max_relative_speed_mps <= 0.0
        {
            return Err("invalid transfer envelope".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub fictional: bool,
    pub fuel_type: String,
    pub fuel_capacity_kg: f64,
    pub dry_mass_kg: f64,
    pub reserve_kg: f64,
    pub minimum_departure_kg: f64,
    pub cargo_capacity_kg: f64,
    pub ammunition_capacity: u32,
    pub home: Uuid,
    #[serde(default)]
    pub alternates: Vec<Uuid>,
    pub compatible_facility_kinds: Vec<FacilityKind>,
    pub cruise_speed_mps: f64,
    pub burn: BurnTable,
    pub climb_burn: BurnTable,
    pub idle_burn: BurnTable,
    pub combat_burn: BurnTable,
    pub combat_phase_seconds: u64,
    pub required_runway_m: f64,
    pub required_runway_width_m: f64,
    pub maximum_takeoff_mass_kg: f64,
    pub maximum_landing_mass_kg: f64,
    pub minimum_departure_hit_points: u32,
    pub intake: Vec<Station>,
    pub outlets: Vec<Station>,
    pub shared_rate_kg_s: f64,
    pub envelope: Envelope,
    pub evidence: Evidence,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stock {
    pub fuel_kg: f64,
    pub ammunition: u32,
    pub repair_points: f64,
}
impl Stock {
    fn valid(&self) -> bool {
        self.fuel_kg.is_finite()
            && self.fuel_kg >= 0.0
            && self.repair_points.is_finite()
            && self.repair_points >= 0.0
    }
    pub fn mass(&self) -> f64 {
        self.fuel_kg + f64::from(self.ammunition) + self.repair_points
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityKind {
    Airfield,
    Port,
    Carrier,
    SupplyShip,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Facility {
    pub fictional: bool,
    pub unit_id: Uuid,
    pub kind: FacilityKind,
    pub fuel_type: String,
    pub stock: Stock,
    pub capacity: Stock,
    pub parking_capacity: usize,
    pub movement_slots: usize,
    pub service_slots: usize,
    pub replenishment_slots: usize,
    pub compatible_profile_ids: Vec<String>,
    pub fuel_rate_kg_s: f64,
    pub ammunition_seconds: f64,
    pub repair_rate_points_s: f64,
    pub cargo_rate_kg_s: f64,
    pub max_sea_state: f64,
    pub min_wind_over_deck_mps: f64,
    pub max_wind_over_deck_mps: f64,
    pub runway_length_m: f64,
    pub runway_width_m: f64,
    pub maximum_operating_mass_kg: f64,
    pub heading_deg: f64,
    pub max_crosswind_mps: f64,
    pub approach: Vec<ApproachOffset>,
    pub movement_seconds: f64,
    pub evidence: Evidence,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproachOffset {
    pub north_m: f64,
    pub east_m: f64,
    pub altitude_m: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitDefinition {
    pub unit_id: Uuid,
    pub profile: Profile,
    pub fuel_kg: f64,
    pub ammunition: u32,
    pub initial_facility: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unavailable {
    pub unit_id: Uuid,
    pub profile_id: String,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub version: u32,
    pub units: Vec<UnitDefinition>,
    pub facilities: Vec<Facility>,
    #[serde(default)]
    pub unavailable: Vec<Unavailable>,
    /// Explicit local/remote report subscriptions; no side-wide stock awareness.
    #[serde(default)]
    pub reports: Vec<ReportRoute>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportRoute {
    pub source: Uuid,
    pub recipient: Uuid,
    pub interval_ticks: u64,
}
impl Configuration {
    pub fn validate(&self, units: &BTreeMap<Uuid, (Side, Domain)>) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported logistics version".into());
        }
        let facilities: BTreeMap<_, _> = self.facilities.iter().map(|f| (f.unit_id, f)).collect();
        if facilities.len() != self.facilities.len() {
            return Err("duplicate logistics facility".into());
        }
        for f in &self.facilities {
            f.evidence.validate_for(f.fictional)?;
            if !units.contains_key(&f.unit_id)
                || f.parking_capacity == 0
                || f.movement_slots == 0
                || f.service_slots == 0
                || !f.stock.valid()
                || !f.capacity.valid()
                || f.stock.fuel_kg > f.capacity.fuel_kg
                || f.stock.ammunition > f.capacity.ammunition
                || f.stock.repair_points > f.capacity.repair_points
                || [
                    f.fuel_rate_kg_s,
                    f.ammunition_seconds,
                    f.repair_rate_points_s,
                    f.cargo_rate_kg_s,
                    f.movement_seconds,
                    f.maximum_operating_mass_kg,
                ]
                .iter()
                .any(|x| !x.is_finite() || *x <= 0.0)
                || [
                    f.max_sea_state,
                    f.min_wind_over_deck_mps,
                    f.max_wind_over_deck_mps,
                    f.runway_length_m,
                    f.runway_width_m,
                    f.heading_deg,
                    f.max_crosswind_mps,
                ]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
                || f.min_wind_over_deck_mps > f.max_wind_over_deck_mps
                || f.approach.is_empty()
                || f.approach.iter().any(|p| {
                    [p.north_m, p.east_m, p.altitude_m]
                        .iter()
                        .any(|v| !v.is_finite())
                })
                || f.fuel_type.is_empty()
            {
                return Err("invalid logistics facility".into());
            }
        }
        let mut seen = BTreeSet::new();
        for u in &self.units {
            let p = &u.profile;
            p.evidence.validate_for(p.fictional)?;
            for evidence in [
                &p.burn.evidence,
                &p.climb_burn.evidence,
                &p.idle_burn.evidence,
                &p.combat_burn.evidence,
                &p.envelope.evidence,
            ] {
                evidence.validate_for(p.fictional)?;
            }
            p.burn.validate()?;
            p.climb_burn.validate()?;
            p.idle_burn.validate()?;
            p.combat_burn.validate()?;
            p.envelope.validate()?;
            if !seen.insert(u.unit_id)
                || !units
                    .get(&u.unit_id)
                    .is_some_and(|(_, d)| matches!(d, Domain::Air | Domain::Sea))
                || p.id.is_empty()
                || p.fuel_type.is_empty()
                || p.compatible_facility_kinds.is_empty()
                || [
                    p.fuel_capacity_kg,
                    p.dry_mass_kg,
                    p.cruise_speed_mps,
                    p.maximum_takeoff_mass_kg,
                    p.maximum_landing_mass_kg,
                ]
                .iter()
                .any(|x| !x.is_finite() || *x <= 0.0)
                || [
                    p.reserve_kg,
                    p.minimum_departure_kg,
                    p.cargo_capacity_kg,
                    p.shared_rate_kg_s,
                    p.required_runway_m,
                    p.required_runway_width_m,
                    u.fuel_kg,
                ]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
                || p.reserve_kg > p.minimum_departure_kg
                || p.minimum_departure_kg > p.fuel_capacity_kg
                || u.fuel_kg > p.fuel_capacity_kg
                || u.ammunition > p.ammunition_capacity
            {
                return Err("invalid logistics unit profile".into());
            }
            for station in p.intake.iter().chain(&p.outlets) {
                if (station.interface == Interface::Underway && units[&u.unit_id].1 != Domain::Sea)
                    || (matches!(station.interface, Interface::Boom | Interface::Drogue)
                        && units[&u.unit_id].1 != Domain::Air)
                {
                    return Err("refueling interface incompatible with unit domain".into());
                }
                station.evidence.validate_for(p.fictional)?;
                if !station.rate_kg_s.is_finite() || station.rate_kg_s <= 0.0 {
                    return Err("invalid refueling rate".into());
                }
            }
            for id in std::iter::once(&p.home)
                .chain(p.alternates.iter())
                .chain(u.initial_facility.iter())
            {
                let f = facilities
                    .get(id)
                    .ok_or("unknown home/alternate facility")?;
                if units[id].0 != units[&u.unit_id].0
                    || f.fuel_type != p.fuel_type
                    || !p.compatible_facility_kinds.contains(&f.kind)
                    || !f.compatible_profile_ids.contains(&p.id)
                {
                    return Err("incompatible home/alternate facility".into());
                }
            }
        }
        for unavailable in &self.unavailable {
            if !units.contains_key(&unavailable.unit_id)
                || seen.contains(&unavailable.unit_id)
                || unavailable.profile_id.is_empty()
                || unavailable.reasons.is_empty()
            {
                return Err("invalid unavailable logistics profile".into());
            }
        }
        let sources: BTreeSet<_> = seen
            .into_iter()
            .chain(facilities.keys().copied())
            .chain(self.unavailable.iter().map(|u| u.unit_id))
            .collect();
        for r in &self.reports {
            if r.interval_ticks == 0
                || !sources.contains(&r.source)
                || !units.contains_key(&r.recipient)
                || units[&r.source].0 != units[&r.recipient].0
            {
                return Err("invalid logistics report route".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Return {
        facility_id: Uuid,
    },
    Depart,
    Cancel,
    Support {
        interface: Interface,
    },
    Refuel {
        donor_id: Uuid,
        interface: Interface,
        amount_kg: f64,
    },
    Cargo {
        source_id: Uuid,
        destination_id: Uuid,
        manifest: Stock,
    },
}
impl PartialEq for Stock {
    fn eq(&self, other: &Self) -> bool {
        self.fuel_kg == other.fuel_kg
            && self.ammunition == other.ammunition
            && self.repair_points == other.repair_points
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Deployed,
    Returning,
    Approach,
    Queued,
    Servicing,
    Ready,
    Departing,
    Rendezvous,
    Connecting,
    Transferring,
    Disconnecting,
    Loading,
    Unloading,
    Replenishing,
    Immobilized,
    Destroyed,
    Unavailable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitStatus {
    pub unit_id: Uuid,
    pub profile_id: String,
    pub fuel_kg: f64,
    pub capacity_kg: f64,
    pub estimated_endurance_seconds: Option<f64>,
    pub receiving_interfaces: Vec<Interface>,
    pub support_interfaces: Vec<Interface>,
    pub support_interface: Option<Interface>,
    pub cargo_capacity_kg: f64,
    pub ammunition: u32,
    pub phase: Phase,
    pub facility_id: Option<Uuid>,
    pub counterpart_id: Option<Uuid>,
    pub cargo: Stock,
    pub transferred_kg: f64,
    pub rate_kg_s: f64,
    pub detail: String,
    pub observed_tick: u64,
    pub received_tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacilityStatus {
    pub facility: Facility,
    pub reserved: Stock,
    pub observed_tick: u64,
    pub received_tick: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Projection {
    pub units: Vec<UnitStatus>,
    pub facilities: Vec<FacilityStatus>,
    pub unavailable: Vec<Unavailable>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UnitRuntime {
    profile: Profile,
    status: UnitStatus,
    support: Option<Interface>,
    requested_kg: f64,
    timer: f64,
    queue_at: u64,
    waypoint: usize,
    cargo_task: Option<(Uuid, Uuid, Stock)>,
    ammo_timer: f64,
    requested_interface: Option<Interface>,
    station: Option<usize>,
    combat_until: u64,
    #[serde(default)]
    cargo_stage: u8,
}
#[derive(Resource, Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Runtime {
    configuration: Option<Configuration>,
    sides: BTreeMap<Uuid, Side>,
    units: BTreeMap<Uuid, UnitRuntime>,
    facilities: BTreeMap<Uuid, Facility>,
    #[serde(with = "crate::snapshot::entries")]
    received: BTreeMap<(Uuid, Uuid), Projection>,
}
impl Runtime {
    pub(super) fn enabled(&self) -> bool {
        self.configuration.is_some()
    }
    fn new(configuration: Configuration) -> Self {
        let units = configuration
            .units
            .iter()
            .map(|u| {
                (
                    u.unit_id,
                    UnitRuntime {
                        profile: u.profile.clone(),
                        status: UnitStatus {
                            unit_id: u.unit_id,
                            profile_id: u.profile.id.clone(),
                            fuel_kg: u.fuel_kg,
                            capacity_kg: u.profile.fuel_capacity_kg,
                            estimated_endurance_seconds: None,
                            receiving_interfaces: u
                                .profile
                                .intake
                                .iter()
                                .map(|s| s.interface)
                                .collect(),
                            support_interfaces: u
                                .profile
                                .outlets
                                .iter()
                                .map(|s| s.interface)
                                .collect(),
                            support_interface: None,
                            cargo_capacity_kg: u.profile.cargo_capacity_kg,
                            ammunition: u.ammunition,
                            phase: if u.initial_facility.is_some() {
                                Phase::Queued
                            } else {
                                Phase::Deployed
                            },
                            facility_id: u.initial_facility,
                            counterpart_id: None,
                            cargo: Stock::default(),
                            transferred_kg: 0.0,
                            rate_kg_s: 0.0,
                            detail: "Awaiting local tasking".into(),
                            observed_tick: 0,
                            received_tick: 0,
                        },
                        support: None,
                        requested_kg: 0.0,
                        timer: 0.0,
                        queue_at: 0,
                        waypoint: 0,
                        cargo_task: None,
                        ammo_timer: 0.0,
                        requested_interface: None,
                        station: None,
                        combat_until: 0,
                        cargo_stage: 0,
                    },
                )
            })
            .collect();
        Self {
            units,
            facilities: configuration
                .facilities
                .iter()
                .map(|f| (f.unit_id, f.clone()))
                .collect(),
            configuration: Some(configuration),
            ..Default::default()
        }
    }
    pub(super) fn ammunition(&self, id: Uuid) -> Option<u32> {
        self.units.get(&id).map(|u| u.status.ammunition)
    }
    pub(super) fn consume_ammunition(&mut self, id: Uuid, tick: u64) -> bool {
        let Some(u) = self.units.get_mut(&id) else {
            return true;
        };
        if u.status.ammunition == 0 {
            return false;
        }
        u.status.ammunition -= 1;
        u.combat_until = tick.saturating_add(u.profile.combat_phase_seconds);
        true
    }
    pub(super) fn home(&self, id: Uuid) -> Option<Uuid> {
        self.units.get(&id).map(|u| u.profile.home)
    }
    pub(super) fn recovered(&self, id: Uuid, facility: Uuid) -> bool {
        self.units.get(&id).is_some_and(|u| {
            u.status.facility_id == Some(facility)
                && matches!(
                    u.status.phase,
                    Phase::Queued | Phase::Servicing | Phase::Ready
                )
        })
    }
    pub(super) fn configured(&self, id: Uuid) -> bool {
        self.units.contains_key(&id)
    }
    pub(super) fn fixed_facility(&self, id: Uuid) -> bool {
        self.facilities
            .get(&id)
            .is_some_and(|f| matches!(f.kind, FacilityKind::Airfield | FacilityKind::Port))
    }
    pub(super) fn movement_allowed(&self, id: Uuid) -> bool {
        self.units.get(&id).is_none_or(|u| {
            u.status.phase == Phase::Deployed && u.status.fuel_kg > u.profile.reserve_kg
        })
    }
    fn compatible(&self, id: Uuid, f: Uuid) -> bool {
        self.units
            .get(&id)
            .zip(self.facilities.get(&f))
            .is_some_and(|(u, f)| {
                self.sides.get(&id) == self.sides.get(&f.unit_id)
                    && u.profile.fuel_type == f.fuel_type
                    && u.profile.compatible_facility_kinds.contains(&f.kind)
                    && f.compatible_profile_ids.contains(&u.profile.id)
            })
    }
    fn cargo_destination(&self, id: Uuid, facility: Uuid) -> bool {
        self.compatible(id, facility)
            || self
                .units
                .get(&id)
                .zip(self.facilities.get(&facility))
                .is_some_and(|(u, f)| {
                    self.sides.get(&id) == self.sides.get(&facility)
                        && f.fuel_type == u.profile.fuel_type
                        && f.kind == FacilityKind::Carrier
                        && f.replenishment_slots > 0
                        && u.profile
                            .outlets
                            .iter()
                            .any(|s| s.interface == Interface::Underway)
                })
    }
    fn gross_mass(&self, id: Uuid) -> f64 {
        let u = &self.units[&id];
        u.profile.dry_mass_kg
            + u.status.fuel_kg
            + u.status.cargo.mass()
            + self.facilities.get(&id).map_or(0.0, |f| f.stock.mass())
            + self.reserved(id).mass()
    }
    fn reserved(&self, id: Uuid) -> Stock {
        let mut stock = Stock::default();
        for u in self.units.values() {
            if let Some((source, _, remaining)) = &u.cargo_task {
                if *source == id {
                    stock.fuel_kg += remaining.fuel_kg;
                    stock.ammunition += remaining.ammunition;
                    stock.repair_points += remaining.repair_points;
                }
            }
        }
        stock
    }
    pub(super) fn command(&mut self, id: Uuid, command: &Command, tick: u64) -> Result<(), String> {
        let current = self.units.get(&id).ok_or("unit logistics unavailable")?;
        if matches!(current.status.phase, Phase::Destroyed | Phase::Unavailable) {
            return Err("unit unavailable".into());
        }
        if matches!(
            current.status.phase,
            Phase::Connecting | Phase::Transferring | Phase::Disconnecting | Phase::Replenishing
        ) && !matches!(command, Command::Cancel)
        {
            return Err("disconnect before assigning another logistics task".into());
        }
        match command {
            Command::Return { facility_id } => {
                if !self.compatible(id, *facility_id) {
                    return Err("incompatible recovery facility".into());
                }
                if current.status.facility_id.is_some()
                    && matches!(
                        current.status.phase,
                        Phase::Queued
                            | Phase::Servicing
                            | Phase::Ready
                            | Phase::Loading
                            | Phase::Unloading
                    )
                {
                    return Err("depart before returning elsewhere".into());
                }
                self.release_reservation(id);
                self.disconnect_donor(id);
                let u = self.units.get_mut(&id).unwrap();
                u.status.phase = Phase::Returning;
                u.status.facility_id = Some(*facility_id);
                u.status.counterpart_id = None;
                u.waypoint = 0;
                u.timer = 0.0;
                u.support = None;
                u.status.detail = "Returning to assigned recovery facility".into();
            }
            Command::Depart => {
                if !matches!(
                    current.status.phase,
                    Phase::Queued | Phase::Servicing | Phase::Ready | Phase::Immobilized
                ) || current.status.facility_id.is_none()
                    || current.status.fuel_kg < current.profile.minimum_departure_kg
                {
                    return Err(
                        "departure requires a berthed unit and minimum operating fuel".into(),
                    );
                }
                let u = self.units.get_mut(&id).unwrap();
                u.status.phase = Phase::Departing;
                u.timer = 0.0;
                u.status.detail =
                    "Departure requested; waiting for operating envelope and slot".into();
            }
            Command::Support { interface } => {
                if current.status.phase != Phase::Deployed
                    || !current
                        .profile
                        .outlets
                        .iter()
                        .any(|s| s.interface == *interface)
                {
                    return Err("unsupported donor interface or donor not deployed".into());
                }
                let u = self.units.get_mut(&id).unwrap();
                u.support = Some(*interface);
                u.status.support_interface = Some(*interface);
                u.status.detail = format!(
                    "{interface:?} refueling support established; waiting for compatible receivers"
                );
            }
            Command::Refuel {
                donor_id,
                interface,
                amount_kg,
            } => {
                let donor = self.units.get(donor_id).ok_or("unknown refueling donor")?;
                if id == *donor_id
                    || self.sides.get(&id) != self.sides.get(donor_id)
                    || !amount_kg.is_finite()
                    || *amount_kg <= 0.0
                    || !(current.status.phase == Phase::Deployed
                        || (current.status.phase == Phase::Immobilized
                            && *interface == Interface::Underway))
                    || donor.status.phase != Phase::Deployed
                    || donor.support != Some(*interface)
                    || current.profile.fuel_type != donor.profile.fuel_type
                    || !current
                        .profile
                        .intake
                        .iter()
                        .any(|s| s.interface == *interface)
                {
                    return Err(
                        "refueling requires compatible, delivered support and receiver tasking"
                            .into(),
                    );
                }
                let u = self.units.get_mut(&id).unwrap();
                u.status.phase = Phase::Rendezvous;
                u.status.counterpart_id = Some(*donor_id);
                u.requested_kg = *amount_kg;
                u.requested_interface = Some(*interface);
                u.station = None;
                u.queue_at = tick;
                u.status.transferred_kg = 0.0;
                u.timer = 0.0;
                u.status.detail = "Rendezvous with assigned donor".into();
            }
            Command::Cargo {
                source_id,
                destination_id,
                manifest,
            } => {
                if source_id == destination_id
                    || !manifest.valid()
                    || manifest.mass() <= 0.0
                    || manifest.mass() > current.profile.cargo_capacity_kg
                    || !self.compatible(id, *source_id)
                    || !self.cargo_destination(id, *destination_id)
                    || current.cargo_task.is_some()
                    || current.status.cargo.mass() > 0.0
                {
                    return Err("invalid cargo task or incompatible facilities".into());
                }
                let f = &self.facilities[source_id];
                if manifest.fuel_kg > f.stock.fuel_kg
                    || manifest.ammunition > f.stock.ammunition
                    || manifest.repair_points > f.stock.repair_points
                {
                    return Err("insufficient source inventory".into());
                }
                // Atomic reservation at executor acceptance, before physical loading.
                let f = self.facilities.get_mut(source_id).unwrap();
                f.stock.fuel_kg -= manifest.fuel_kg;
                f.stock.ammunition -= manifest.ammunition;
                f.stock.repair_points -= manifest.repair_points;
                self.disconnect_donor(id);
                let u = self.units.get_mut(&id).unwrap();
                u.cargo_task = Some((*source_id, *destination_id, manifest.clone()));
                u.status.phase = Phase::Returning;
                u.status.facility_id = Some(*source_id);
                u.waypoint = 0;
                u.timer = 0.0;
                u.support = None;
                u.status.counterpart_id = None;
                u.status.detail = "Cargo reserved; returning to load".into();
            }
            Command::Cancel => {
                self.release_reservation(id);
                self.disconnect_donor(id);
                let u = self.units.get_mut(&id).unwrap();
                u.support = None;
                if u.status.phase == Phase::Replenishing {
                    u.cargo_stage = 2;
                    u.timer = 0.0;
                } else if matches!(
                    u.status.phase,
                    Phase::Connecting | Phase::Transferring | Phase::Rendezvous
                ) {
                    u.status.phase = Phase::Disconnecting;
                    u.timer = 0.0;
                } else if u.status.facility_id.is_some()
                    && matches!(
                        u.status.phase,
                        Phase::Loading | Phase::Unloading | Phase::Departing
                    )
                {
                    u.status.phase = Phase::Queued;
                    u.queue_at = tick;
                    u.timer = 0.0;
                } else if matches!(u.status.phase, Phase::Returning | Phase::Approach) {
                    u.status.phase = Phase::Deployed;
                    u.status.facility_id = None;
                }
                u.status.detail = "Task cancelled; completed transfers retained".into();
            }
        }
        Ok(())
    }
    fn release_reservation(&mut self, id: Uuid) {
        if let Some((source, _, remaining)) =
            self.units.get_mut(&id).and_then(|u| u.cargo_task.take())
        {
            let f = self.facilities.get_mut(&source).unwrap();
            f.stock.fuel_kg += remaining.fuel_kg;
            f.stock.ammunition += remaining.ammunition;
            f.stock.repair_points += remaining.repair_points;
        }
    }
    fn disconnect_donor(&mut self, id: Uuid) {
        for u in self
            .units
            .values_mut()
            .filter(|u| u.status.counterpart_id == Some(id))
        {
            u.status.phase = Phase::Disconnecting;
            u.timer = 0.0;
            u.status.rate_kg_s = 0.0;
            u.status.detail = "Donor support ended".into();
        }
    }
    fn local(&self, id: Uuid, tick: u64) -> Projection {
        let mut projection = Projection::default();
        if let Some(u) = self.units.get(&id) {
            let mut s = u.status.clone();
            s.support_interface = u.support;
            s.observed_tick = tick;
            s.received_tick = tick;
            projection.units.push(s);
        }
        if let Some(f) = self.facilities.get(&id) {
            projection.facilities.push(FacilityStatus {
                facility: f.clone(),
                reserved: self.reserved(id),
                observed_tick: tick,
                received_tick: tick,
            });
        }
        if let Some(c) = &self.configuration {
            projection.unavailable = c
                .unavailable
                .iter()
                .filter(|u| u.unit_id == id)
                .cloned()
                .collect();
        }
        projection
    }
    pub(super) fn projection(&self, id: Uuid, tick: u64) -> Option<Projection> {
        self.configuration.as_ref()?;
        let mut projection = self.local(id, tick);
        for ((recipient, _), report) in &self.received {
            if *recipient == id
                && report.units.iter().all(|u| u.unit_id != id)
                && report.facilities.iter().all(|f| f.facility.unit_id != id)
            {
                projection.units.extend(report.units.clone());
                projection.facilities.extend(report.facilities.clone());
                projection.unavailable.extend(report.unavailable.clone());
            }
        }
        Some(projection)
    }
}

#[derive(Clone)]
struct Position {
    pose: GeoPose,
    velocity: Velocity,
    side: Side,
    domain: Domain,
    destroyed: bool,
}
fn positions(world: &mut World) -> BTreeMap<Uuid, Position> {
    let health = world.resource::<CombatState>().clone();
    let mut query = world.query::<(
        &SimEntityId,
        &GeoPose,
        &Velocity,
        &Ownership,
        &DomainKind,
        &operations::CombatState,
    )>();
    query
        .iter(world)
        .map(|(id, p, v, s, d, c)| {
            (
                id.0,
                Position {
                    pose: *p,
                    velocity: *v,
                    side: s.0,
                    domain: d.0,
                    destroyed: c.destroyed || health.hit_points(id.0) == Some(0),
                },
            )
        })
        .collect()
}
fn condition(world: &World, pose: GeoPose, tick: u64) -> sensing::Weather {
    world
        .get_resource::<sensing::Runtime>()
        .map(|r| r.local_view(Uuid::nil(), pose, tick).weather)
        .unwrap_or_default()
}
fn facility_open(f: &Facility, p: &Position, weather: &sensing::Weather) -> bool {
    let heading = if p.velocity.north_mps.hypot(p.velocity.east_mps) > 0.1 {
        p.velocity.east_mps.atan2(p.velocity.north_mps)
    } else {
        f.heading_deg.to_radians()
    };
    let north = p.velocity.north_mps - weather.wind_north_mps;
    let east = p.velocity.east_mps - weather.wind_east_mps;
    let wind = north * heading.cos() + east * heading.sin();
    let crosswind = (-north * heading.sin() + east * heading.cos()).abs();
    !p.destroyed
        && crosswind <= f.max_crosswind_mps
        && weather.sea_state <= f.max_sea_state
        && (f.kind != FacilityKind::Carrier
            || (wind >= f.min_wind_over_deck_mps && wind <= f.max_wind_over_deck_mps))
}
fn operating_speed(p: &Position, w: &sensing::Weather) -> f64 {
    let (north, east) = if p.domain == Domain::Air {
        (w.wind_north_mps, w.wind_east_mps)
    } else {
        (0.0, 0.0)
    };
    (p.velocity.north_mps - north).hypot(p.velocity.east_mps - east)
}
fn transfer_open(
    receiver: &Position,
    donor: &Position,
    e: &Envelope,
    w: &sensing::Weather,
) -> bool {
    let speed = operating_speed(donor, w);
    let receiver_speed = operating_speed(receiver, w);
    !receiver.destroyed
        && !donor.destroyed
        && receiver.side == donor.side
        && donor.pose.altitude_m >= e.min_altitude_m
        && donor.pose.altitude_m <= e.max_altitude_m
        && receiver.pose.altitude_m >= e.min_altitude_m
        && receiver.pose.altitude_m <= e.max_altitude_m
        && receiver_speed >= e.min_speed_mps
        && receiver_speed <= e.max_speed_mps
        && speed >= e.min_speed_mps
        && speed <= e.max_speed_mps
        && geodesy::slant_distance_m(receiver.pose, donor.pose) <= e.connection_distance_m
        && (receiver.velocity.north_mps - donor.velocity.north_mps)
            .hypot(receiver.velocity.east_mps - donor.velocity.east_mps)
            <= e.max_relative_speed_mps
        && w.sea_state <= e.max_sea_state
}
fn guide(from: GeoPose, to: GeoPose, speed: f64) -> Velocity {
    let north = (to.latitude_deg - from.latitude_deg) * 111_320.0;
    let delta_lon = (to.longitude_deg - from.longitude_deg + 180.0).rem_euclid(360.0) - 180.0;
    let east = delta_lon * 111_320.0 * from.latitude_deg.to_radians().cos();
    let length = north.hypot(east).max(1.0);
    Velocity {
        north_mps: north / length * speed,
        east_mps: east / length * speed,
        climb_mps: (to.altitude_m - from.altitude_m).clamp(-30.0, 30.0),
    }
}

/// Guidance follows delivered tasks. Attached platforms use the current moving facility.
pub(super) fn guide_units(world: &mut World) {
    if world.resource::<Runtime>().units.is_empty() {
        return;
    }
    let tick = world.resource::<SimClock>().tick;
    let positions = positions(world);
    let mut runtime = world.remove_resource::<Runtime>().unwrap();
    let mut motion = BTreeMap::new();
    let mut attached = BTreeMap::new();
    let ids: Vec<_> = runtime.units.keys().copied().collect();
    for id in ids {
        let p = &positions[&id];
        let carried_on_lost_carrier = runtime.units.get(&id).is_some_and(|u| {
            matches!(
                u.status.phase,
                Phase::Queued
                    | Phase::Servicing
                    | Phase::Ready
                    | Phase::Loading
                    | Phase::Unloading
                    | Phase::Departing
            ) && u
                .status
                .facility_id
                .and_then(|f| runtime.facilities.get(&f))
                .is_some_and(|f| f.kind == FacilityKind::Carrier && positions[&f.unit_id].destroyed)
        });
        if p.destroyed || carried_on_lost_carrier {
            runtime.release_reservation(id);
            runtime.disconnect_donor(id);
            let u = runtime.units.get_mut(&id).unwrap();
            u.status.phase = Phase::Destroyed;
            u.status.cargo = Stock::default();
            u.status.detail = "Platform destroyed; carried cargo lost".into();
        }
        if p.domain == Domain::Sea
            && runtime.units[&id].status.fuel_kg == 0.0
            && runtime.units[&id].status.phase == Phase::Deployed
        {
            runtime.units.get_mut(&id).unwrap().status.phase = Phase::Immobilized;
        }
        let u = &runtime.units[&id];
        if matches!(u.status.phase, Phase::Deployed) && u.status.fuel_kg <= u.profile.reserve_kg {
            let home = std::iter::once(u.profile.home)
                .chain(u.profile.alternates.iter().copied())
                .find(|f| {
                    positions
                        .get(f)
                        .zip(runtime.facilities.get(f))
                        .is_some_and(|(p, f)| facility_open(f, p, &condition(world, p.pose, tick)))
                });
            if let Some(f) = home {
                let u = runtime.units.get_mut(&id).unwrap();
                u.status.phase = Phase::Returning;
                u.status.facility_id = Some(f);
                u.waypoint = 0;
                u.status.detail = "Reserve reached; automatic return".into();
                u.support = None;
                runtime.disconnect_donor(id);
            }
        }
        let u = runtime.units.get_mut(&id).unwrap();
        match u.status.phase {
            Phase::Queued
            | Phase::Servicing
            | Phase::Ready
            | Phase::Loading
            | Phase::Unloading
            | Phase::Departing => {
                if let Some(f) = u.status.facility_id.and_then(|f| positions.get(&f)) {
                    attached.insert(id, f.pose);
                    motion.insert(id, Velocity::default());
                }
            }
            Phase::Returning | Phase::Approach => {
                if let Some(f) = u.status.facility_id.and_then(|f| positions.get(&f)) {
                    let facility = &runtime.facilities[&u.status.facility_id.unwrap()];
                    let offset = &facility.approach[u.waypoint.min(facility.approach.len() - 1)];
                    // Approach waypoints are local metre offsets, carried by the facility.
                    let target = geodesy::advance_position(
                        f.pose,
                        offset.north_m,
                        offset.east_m,
                        offset.altitude_m,
                    )
                    .unwrap_or(f.pose);
                    motion.insert(id, guide(p.pose, target, u.profile.cruise_speed_mps));
                }
            }
            Phase::Rendezvous | Phase::Connecting | Phase::Transferring | Phase::Disconnecting => {
                if let Some(d) = u.status.counterpart_id.and_then(|d| positions.get(&d)) {
                    motion.insert(
                        id,
                        if p.domain == Domain::Sea && u.status.fuel_kg == 0.0 {
                            Velocity::default()
                        } else if u.status.phase == Phase::Rendezvous
                            && geodesy::slant_distance_m(p.pose, d.pose)
                                > u.profile.envelope.connection_distance_m
                        {
                            guide(p.pose, d.pose, u.profile.cruise_speed_mps)
                        } else {
                            d.velocity
                        },
                    );
                }
            }
            Phase::Replenishing => {
                if let Some(parent) = u.status.facility_id.and_then(|f| positions.get(&f)) {
                    motion.insert(id, parent.velocity);
                }
            }
            Phase::Destroyed | Phase::Immobilized | Phase::Unavailable => {
                motion.insert(id, Velocity::default());
            }
            Phase::Deployed => {}
        }
    }
    let mut query = world.query::<(
        &SimEntityId,
        &mut GeoPose,
        &mut Velocity,
        &mut GeodesicMotionState,
        Option<&mut CyclicFlightPathState>,
    )>();
    for (id, mut pose, mut v, mut origin, path) in query.iter_mut(world) {
        if let Some(next) = motion.get(&id.0) {
            *v = *next;
            if let Some(mut path) = path {
                path.active = false;
            }
        }
        if let Some(p) = attached.get(&id.0) {
            *pose = *p;
            origin.origin = *p;
            origin.velocity = Velocity::default();
        }
    }
    world.insert_resource(runtime);
}

/// Inventory transitions happen after movement and once per frame using exact game delta.
pub(super) fn advance(world: &mut World) {
    if world.resource::<Runtime>().units.is_empty() {
        return;
    }
    let tick = world.resource::<SimClock>().tick;
    let dt = world
        .get_resource::<FrameClock>()
        .map_or(1.0, |f| f64::from(f.delta_units) / 240.0);
    let positions = positions(world);
    let mut runtime = world.remove_resource::<Runtime>().unwrap();
    let ids: Vec<_> = runtime.units.keys().copied().collect();
    let training = world.resource::<CombatState>().clone();
    let mut query = world.query::<(&SimEntityId, &operations::CombatState)>();
    let ammunition: BTreeMap<_, _> = query
        .iter(world)
        .map(|(id, c)| {
            (
                id.0,
                training
                    .weapon_status(id.0)
                    .map_or(c.profile.ammunition, |w| w.ammunition),
            )
        })
        .collect();
    for id in &ids {
        let p = &positions[id];
        let gross_mass = runtime.gross_mass(*id);
        let u = runtime.units.get_mut(id).unwrap();
        if ammunition[id] < u.status.ammunition {
            u.combat_until = tick.saturating_add(u.profile.combat_phase_seconds);
        }
        u.status.ammunition = u.status.ammunition.min(ammunition[id]);
        if u.status.phase == Phase::Ready
            && training
                .hit_points(*id)
                .zip(training.maximum_hit_points(*id))
                .is_some_and(|(hp, max)| hp < max)
        {
            u.status.phase = Phase::Queued;
            u.queue_at = tick;
        }
        u.status.rate_kg_s = 0.0;
        u.status.observed_tick = tick;
        u.status.received_tick = tick;
        let attached = matches!(
            u.status.phase,
            Phase::Queued
                | Phase::Servicing
                | Phase::Ready
                | Phase::Loading
                | Phase::Unloading
                | Phase::Departing
        );
        if attached {
            u.status.estimated_endurance_seconds = None;
        }
        let rescue = p.domain == Domain::Sea
            && u.status.fuel_kg == 0.0
            && matches!(
                u.status.phase,
                Phase::Rendezvous | Phase::Connecting | Phase::Transferring | Phase::Disconnecting
            );
        if !attached
            && !rescue
            && !matches!(
                u.status.phase,
                Phase::Destroyed | Phase::Unavailable | Phase::Immobilized
            )
        {
            let w = condition(world, p.pose, tick);
            let speed = operating_speed(p, &w);
            let mass = gross_mass;
            let table = if tick < u.combat_until {
                &u.profile.combat_burn
            } else if p.velocity.climb_mps > 0.1 {
                &u.profile.climb_burn
            } else if speed < 1.0 {
                &u.profile.idle_burn
            } else {
                &u.profile.burn
            };
            match table.rate(p.pose.altitude_m, speed, mass) {
                Some(rate) => {
                    u.status.fuel_kg = (u.status.fuel_kg - rate * dt).max(0.0);
                    u.status.estimated_endurance_seconds =
                        (rate > 0.0).then(|| u.status.fuel_kg / rate);
                }
                None => {
                    u.status.estimated_endurance_seconds = None;
                    u.status.phase = Phase::Unavailable;
                    u.status.detail =
                        "Outside sourced fuel-performance envelope; operation unavailable".into();
                }
            }
            if u.status.fuel_kg == 0.0 {
                u.status.phase = if p.domain == Domain::Air {
                    Phase::Destroyed
                } else {
                    Phase::Immobilized
                };
                u.status.detail = "Fuel exhausted".into();
                u.support = None;
            }
        }
    }
    // Stable FIFO order for shared physical resources.
    let mut order = ids.clone();
    order.sort_by_key(|id| (runtime.units[id].queue_at, *id));
    let mut movement_slots: BTreeMap<Uuid, usize> = BTreeMap::new();
    let mut service_slots: BTreeMap<Uuid, usize> = BTreeMap::new();
    let mut replenishment_slots: BTreeMap<Uuid, usize> = BTreeMap::new();

    let mut pump_used: BTreeMap<Uuid, f64> = BTreeMap::new();
    for id in order {
        let p = &positions[&id];
        let phase = runtime.units[&id].status.phase;
        if matches!(phase, Phase::Returning | Phase::Approach) {
            let facility_id = runtime.units[&id].status.facility_id.unwrap();
            let f = &runtime.facilities[&facility_id];
            let fp = &positions[&facility_id];
            let w = condition(world, fp.pose, tick);
            let u = &runtime.units[&id];
            let mass = runtime.gross_mass(id);
            if mass > u.profile.maximum_landing_mass_kg
                || mass > f.maximum_operating_mass_kg
                || (f.kind == FacilityKind::Airfield
                    && (f.runway_length_m < u.profile.required_runway_m
                        || f.runway_width_m < u.profile.required_runway_width_m))
            {
                runtime.units.get_mut(&id).unwrap().status.detail =
                    "Recovery blocked: runway or operating mass limit".into();
                continue;
            }
            if p.domain == Domain::Sea
                && f.kind == FacilityKind::Carrier
                && u.status.cargo.mass() > 0.0
            {
                if transfer_open(p, fp, &u.profile.envelope, &w) {
                    let u = runtime.units.get_mut(&id).unwrap();
                    u.status.phase = Phase::Replenishing;
                    u.cargo_stage = 0;
                    u.timer = 0.0;
                    u.status.detail = "Connecting underway cargo equipment".into();
                }
                continue;
            }
            let occupied = runtime
                .units
                .values()
                .filter(|u| {
                    u.status.facility_id == Some(facility_id)
                        && matches!(
                            u.status.phase,
                            Phase::Queued
                                | Phase::Servicing
                                | Phase::Ready
                                | Phase::Loading
                                | Phase::Unloading
                                | Phase::Departing
                        )
                })
                .count();
            let u = runtime.units.get_mut(&id).unwrap();
            let offset = &f.approach[u.waypoint.min(f.approach.len() - 1)];
            let target = geodesy::advance_position(
                fp.pose,
                offset.north_m,
                offset.east_m,
                offset.altitude_m,
            )
            .unwrap_or(fp.pose);
            if geodesy::slant_distance_m(p.pose, target) <= u.profile.cruise_speed_mps * dt + 10.0 {
                if u.waypoint + 1 < f.approach.len() {
                    u.waypoint += 1;
                } else if !facility_open(f, fp, &w) || occupied >= f.parking_capacity {
                    u.status.detail = "Recovery holding: weather or parking capacity".into();
                } else {
                    let slots = movement_slots.entry(facility_id).or_default();
                    if *slots < f.movement_slots {
                        *slots += 1;
                        u.status.phase = Phase::Approach;
                        u.timer += dt;
                        if u.timer + 1e-9 >= f.movement_seconds {
                            u.status.phase = if u.cargo_task.is_some() {
                                Phase::Loading
                            } else if u.status.cargo.mass() > 0.0 {
                                Phase::Unloading
                            } else {
                                Phase::Queued
                            };
                            u.queue_at = tick;
                            u.timer = 0.0;
                            u.status.detail = "Arrived; awaiting local service".into();
                        }
                    }
                }
            }
        } else if matches!(
            phase,
            Phase::Queued
                | Phase::Servicing
                | Phase::Ready
                | Phase::Departing
                | Phase::Loading
                | Phase::Unloading
        ) {
            let facility_id = runtime.units[&id].status.facility_id.unwrap();
            let reserved = runtime.reserved(facility_id);
            let gross_mass = runtime.gross_mass(id);
            let f = runtime.facilities.get_mut(&facility_id).unwrap();
            let fp = &positions[&facility_id];
            let w = condition(world, fp.pose, tick);
            let u = runtime.units.get_mut(&id).unwrap();
            if !facility_open(f, fp, &w) {
                u.status.detail = "Facility operations suspended by damage or weather".into();
                continue;
            }
            if phase == Phase::Departing {
                let mass = gross_mass;
                if mass > u.profile.maximum_takeoff_mass_kg
                    || mass > f.maximum_operating_mass_kg
                    || world
                        .resource::<CombatState>()
                        .hit_points(id)
                        .is_some_and(|hp| hp < u.profile.minimum_departure_hit_points)
                    || (f.kind == FacilityKind::Airfield
                        && (f.runway_length_m < u.profile.required_runway_m
                            || f.runway_width_m < u.profile.required_runway_width_m))
                {
                    u.status.detail =
                        "Departure blocked: runway, mass or minimum health requirement".into();
                    continue;
                }
                let slots = movement_slots.entry(facility_id).or_default();
                if *slots < f.movement_slots {
                    *slots += 1;
                    u.timer += dt;
                    if u.timer + 1e-9 >= f.movement_seconds {
                        u.status.phase = Phase::Deployed;
                        u.status.facility_id = None;
                        u.timer = 0.0;
                        u.status.detail = "Departed".into();
                    }
                }
                continue;
            }
            if phase == Phase::Ready {
                continue;
            }
            let slots = service_slots.entry(facility_id).or_default();
            if *slots >= f.service_slots {
                u.status.detail = "Waiting for servicing capacity".into();
                continue;
            }
            *slots += 1;
            if phase == Phase::Loading {
                if let Some((_, destination, remaining)) = &mut u.cargo_task {
                    u.timer += f.cargo_rate_kg_s * dt;
                    u.timer = transfer_stock(remaining, &mut u.status.cargo, None, u.timer);
                    if remaining.mass() <= 1e-9 {
                        u.status.phase = Phase::Returning;
                        u.status.facility_id = Some(*destination);
                        u.waypoint = 0;
                        u.timer = 0.0;
                        u.status.detail = "Cargo loaded; en route".into();
                        u.cargo_task = None;
                    }
                }
                continue;
            }
            if phase == Phase::Unloading {
                u.timer += f.cargo_rate_kg_s * dt;
                let capacity = Stock {
                    fuel_kg: (f.capacity.fuel_kg - reserved.fuel_kg).max(0.0),
                    ammunition: f.capacity.ammunition.saturating_sub(reserved.ammunition),
                    repair_points: (f.capacity.repair_points - reserved.repair_points).max(0.0),
                };
                u.timer =
                    transfer_stock(&mut u.status.cargo, &mut f.stock, Some(&capacity), u.timer);
                u.status.detail = "Unloading cargo; destination capacity enforced".into();
                if u.status.cargo.mass() <= 1e-9 {
                    u.status.phase = Phase::Queued;
                    u.queue_at = tick;
                    u.timer = 0.0;
                }
                continue;
            }
            u.status.phase = Phase::Servicing;
            let fuel = (u.profile.fuel_capacity_kg - u.status.fuel_kg)
                .min(f.fuel_rate_kg_s * dt)
                .min(f.stock.fuel_kg);
            u.status.fuel_kg += fuel;
            f.stock.fuel_kg -= fuel;
            if u.status.ammunition < u.profile.ammunition_capacity && f.stock.ammunition > 0 {
                u.ammo_timer += dt;
                let ammo = ((u.ammo_timer + 1e-9) / f.ammunition_seconds).floor() as u32;
                let ammo = ammo
                    .min(f.stock.ammunition)
                    .min(u.profile.ammunition_capacity - u.status.ammunition);
                u.status.ammunition += ammo;
                f.stock.ammunition -= ammo;
                u.ammo_timer = (u.ammo_timer - f64::from(ammo) * f.ammunition_seconds).max(0.0);
            }
            let hp = world.resource::<CombatState>().hit_points(id);
            let max_hp = runtime
                .configuration
                .as_ref()
                .and_then(|_| world.resource::<CombatState>().maximum_hit_points(id));
            if let Some((hp, max)) = hp.zip(max_hp) {
                if hp > 0 && hp < max {
                    u.timer += f.repair_rate_points_s * dt;
                    let repair = u
                        .timer
                        .floor()
                        .min(f.stock.repair_points)
                        .min(f64::from(max - hp)) as u32;
                    if repair > 0 {
                        world.resource_mut::<CombatState>().repair(id, repair);
                        f.stock.repair_points -= f64::from(repair);
                        u.timer -= f64::from(repair);
                    }
                }
            }
            let repaired = hp.zip(max_hp).is_none_or(|(hp, max)| hp == max);
            if u.status.fuel_kg >= u.profile.fuel_capacity_kg - 1e-9
                && u.status.ammunition == u.profile.ammunition_capacity
                && repaired
            {
                u.status.phase = Phase::Ready;
                u.status.detail = "Full turnaround complete".into();
            } else {
                u.status.detail =
                    "Automatic turnaround; depleted supplies can delay completion".into();
            }
        } else if phase == Phase::Replenishing {
            let destination = runtime.units[&id].status.facility_id.unwrap();
            let target = &positions[&destination];
            let reserved = runtime.reserved(destination);
            let f = runtime.facilities.get_mut(&destination).unwrap();
            let u = runtime.units.get_mut(&id).unwrap();
            let weather = condition(world, target.pose, tick);
            if u.cargo_stage < 2 && !transfer_open(p, target, &u.profile.envelope, &weather) {
                u.cargo_stage = 2;
                u.timer = 0.0;
                u.status.detail = "Cargo transfer interrupted by operating envelope".into();
            }
            let occupied = replenishment_slots.entry(destination).or_default();
            if *occupied >= f.replenishment_slots {
                u.status.detail = "Waiting for receiver replenishment station".into();
                continue;
            }
            *occupied += 1;
            if u.cargo_stage == 0 {
                u.timer += dt;
                if u.timer + 1e-9 >= u.profile.envelope.connection_seconds {
                    u.cargo_stage = 1;
                    u.timer = 0.0;
                }
            } else if u.cargo_stage == 2 {
                u.timer += dt;
                if u.timer + 1e-9 >= u.profile.envelope.disconnection_seconds {
                    u.status.phase = Phase::Deployed;
                    u.status.facility_id = None;
                    u.timer = 0.0;
                    u.status.detail =
                        "Underway cargo equipment disconnected; remaining cargo retained".into();
                }
            } else {
                let rate = u
                    .profile
                    .outlets
                    .iter()
                    .filter(|s| s.interface == Interface::Underway)
                    .map(|s| s.rate_kg_s)
                    .fold(0.0, f64::max)
                    .min(u.profile.shared_rate_kg_s)
                    .min(f.cargo_rate_kg_s);
                let before = u.status.cargo.fuel_kg;
                let capacity = Stock {
                    fuel_kg: (f.capacity.fuel_kg - reserved.fuel_kg).max(0.0),
                    ammunition: f.capacity.ammunition.saturating_sub(reserved.ammunition),
                    repair_points: (f.capacity.repair_points - reserved.repair_points).max(0.0),
                };
                u.timer = transfer_stock(
                    &mut u.status.cargo,
                    &mut f.stock,
                    Some(&capacity),
                    rate * dt + u.timer,
                );
                let amount = before - u.status.cargo.fuel_kg;
                u.status.transferred_kg += amount;
                u.status.rate_kg_s = amount / dt;
                u.status.detail =
                    "Underway delivery into carrier fuel, ammunition and repair stocks".into();
                if u.status.cargo.mass() <= 1e-9 {
                    u.cargo_stage = 2;
                    u.timer = 0.0;
                }
            }
        } else if matches!(
            phase,
            Phase::Rendezvous | Phase::Connecting | Phase::Transferring
        ) {
            let donor_id = runtime.units[&id].status.counterpart_id.unwrap();
            let donor = runtime.units[&donor_id].clone();
            let dp = &positions[&donor_id];
            let w = condition(world, dp.pose, tick);
            let Some(interface) = donor
                .support
                .filter(|i| Some(*i) == runtime.units[&id].requested_interface)
            else {
                let u = runtime.units.get_mut(&id).unwrap();
                u.status.phase = Phase::Disconnecting;
                u.timer = 0.0;
                continue;
            };
            if donor.status.phase != Phase::Deployed
                || !transfer_open(p, dp, &donor.profile.envelope, &w)
                || !transfer_open(p, dp, &runtime.units[&id].profile.envelope, &w)
            {
                let u = runtime.units.get_mut(&id).unwrap();
                if phase != Phase::Rendezvous {
                    u.status.phase = Phase::Disconnecting;
                    u.timer = 0.0;
                }
                u.status.detail = "Transfer interrupted: outside operating envelope".into();
                continue;
            }
            let stations: Vec<_> = donor
                .profile
                .outlets
                .iter()
                .filter(|s| s.interface == interface)
                .collect();
            let assigned = runtime.units[&id].station;
            let occupied: BTreeSet<_> = runtime
                .units
                .values()
                .filter(|u| u.status.counterpart_id == Some(donor_id) && u.status.unit_id != id)
                .filter_map(|u| u.station)
                .collect();
            let Some(slot) =
                assigned.or_else(|| (0..stations.len()).find(|i| !occupied.contains(i)))
            else {
                runtime.units.get_mut(&id).unwrap().status.detail =
                    "Waiting for donor station".into();
                continue;
            };
            if slot >= stations.len() {
                continue;
            }
            let station = stations[slot];
            let u = runtime.units.get_mut(&id).unwrap();
            if phase == Phase::Rendezvous {
                u.status.phase = Phase::Connecting;
                u.timer = 0.0;
                u.station = Some(slot);
            }
            if matches!(u.status.phase, Phase::Connecting) {
                u.timer += dt;
                if u.timer
                    >= donor
                        .profile
                        .envelope
                        .connection_seconds
                        .max(u.profile.envelope.connection_seconds)
                {
                    u.status.phase = Phase::Transferring;
                    u.timer = 0.0;
                }
                continue;
            }
            let intake = u
                .profile
                .intake
                .iter()
                .find(|s| s.interface == interface)
                .unwrap()
                .rate_kg_s;
            let used = pump_used.entry(donor_id).or_default();
            let rate = station
                .rate_kg_s
                .min(intake)
                .min((donor.profile.shared_rate_kg_s - *used).max(0.0));
            *used += rate;
            let underway = interface == Interface::Underway;
            let available = if underway {
                runtime
                    .facilities
                    .get(&donor_id)
                    .map_or(0.0, |f| f.stock.fuel_kg)
            } else {
                (donor.status.fuel_kg - donor.profile.reserve_kg).max(0.0)
            };
            let amount = (rate * dt)
                .min(u.requested_kg - u.status.transferred_kg)
                .min(u.profile.fuel_capacity_kg - u.status.fuel_kg)
                .min(available)
                .max(0.0);
            u.status.fuel_kg += amount;
            u.status.transferred_kg += amount;
            u.status.rate_kg_s = amount / dt;
            u.status.detail = "Fuel transfer active; both aircraft continue burning fuel".into();
            if amount == 0.0
                || u.status.transferred_kg >= u.requested_kg - 1e-9
                || u.status.fuel_kg >= u.profile.fuel_capacity_kg - 1e-9
                || amount >= available - 1e-9
            {
                u.status.phase = Phase::Disconnecting;
                u.timer = 0.0;
                u.status.rate_kg_s = 0.0;
            }
            if underway {
                if let Some(f) = runtime.facilities.get_mut(&donor_id) {
                    f.stock.fuel_kg -= amount;
                }
            } else {
                runtime.units.get_mut(&donor_id).unwrap().status.fuel_kg -= amount;
            }
        } else if phase == Phase::Disconnecting {
            let u = runtime.units.get_mut(&id).unwrap();
            u.timer += dt;
            if u.timer + 1e-9 >= u.profile.envelope.disconnection_seconds {
                u.status.phase = Phase::Deployed;
                u.status.counterpart_id = None;
                u.station = None;
                u.requested_interface = None;
                u.timer = 0.0;
                u.status.detail = "Disconnected; completed transfer retained".into();
            }
        }
    }
    for id in &ids {
        if matches!(
            runtime.units[id].status.phase,
            Phase::Destroyed | Phase::Unavailable
        ) {
            runtime.release_reservation(*id);
            runtime.disconnect_donor(*id);
        }
    }
    let mut attached_query =
        world.query::<(&SimEntityId, &mut GeoPose, &mut GeodesicMotionState)>();
    for (id, mut pose, mut motion) in attached_query.iter_mut(world) {
        if let Some(u) = runtime.units.get(&id.0) {
            if matches!(
                u.status.phase,
                Phase::Queued
                    | Phase::Servicing
                    | Phase::Ready
                    | Phase::Loading
                    | Phase::Unloading
                    | Phase::Departing
            ) {
                if let Some(parent) = u.status.facility_id.and_then(|f| positions.get(&f)) {
                    *pose = parent.pose;
                    motion.origin = parent.pose;
                    motion.velocity = Velocity::default();
                }
            }
        }
    }
    let mut query = world.query::<(&SimEntityId, &mut operations::CombatState, &mut Velocity)>();
    for (id, mut combat, mut v) in query.iter_mut(world) {
        if let Some(u) = runtime.units.get(&id.0) {
            combat.profile.ammunition = u.status.ammunition;
            combat.profile.fuel_seconds = if u.status.fuel_kg > 0.0 {
                u.status
                    .estimated_endurance_seconds
                    .unwrap_or(u.status.fuel_kg)
            } else {
                0.0
            };
            if u.status.phase == Phase::Destroyed {
                combat.destroyed = true;
            }
            if matches!(
                u.status.phase,
                Phase::Destroyed | Phase::Unavailable | Phase::Immobilized
            ) {
                *v = Velocity::default();
            }
        }
    }
    for (id, u) in &runtime.units {
        world
            .resource_mut::<CombatState>()
            .set_ammunition(*id, u.status.ammunition);
    }
    world.insert_resource(runtime);
}

impl Simulation {
    pub fn configure_logistics(&mut self, configuration: Configuration) -> Result<(), String> {
        if self.world.resource::<Runtime>().enabled() {
            return Err("logistics already configured".into());
        }
        let mut query = self
            .world
            .query::<(&SimEntityId, &Ownership, &DomainKind)>();
        let units = query
            .iter(&self.world)
            .map(|(id, s, d)| (id.0, (s.0, d.0)))
            .collect();
        configuration.validate(&units)?;
        self.world
            .resource_mut::<CombatState>()
            .initialize_logistics_health();
        let mut runtime = Runtime::new(configuration);
        runtime.sides = units.iter().map(|(id, (side, _))| (*id, *side)).collect();
        self.world.insert_resource(runtime);
        let ammunition: Vec<_> = self
            .world
            .resource::<Runtime>()
            .units
            .iter()
            .map(|(id, u)| (*id, u.status.ammunition))
            .collect();
        for (id, n) in ammunition {
            self.world
                .resource_mut::<CombatState>()
                .set_ammunition(id, n);
            let mut q = self
                .world
                .query::<(&SimEntityId, &mut operations::CombatState)>();
            for (unit, mut state) in q.iter_mut(&mut self.world) {
                if unit.0 == id {
                    state.profile.ammunition = n;
                }
            }
        }
        Ok(())
    }
    pub fn logistics_projection(&self, owner: Uuid) -> Option<Projection> {
        self.world
            .resource::<Runtime>()
            .projection(owner, self.tick())
    }
    pub fn local_logistics_report(&self, source: Uuid) -> Option<Projection> {
        let r = self.world.resource::<Runtime>();
        r.configuration.as_ref()?;
        Some(r.local(source, self.tick()))
    }
    pub fn logistics_report_routes(&self) -> Vec<ReportRoute> {
        self.world
            .resource::<Runtime>()
            .configuration
            .as_ref()
            .map(|c| c.reports.clone())
            .unwrap_or_default()
    }
    pub fn receive_logistics_report(
        &mut self,
        recipient: Uuid,
        source: Uuid,
        mut report: Projection,
    ) -> bool {
        let tick = self.tick();
        let runtime = self.world.resource_mut::<Runtime>();
        let mut runtime = runtime;
        if !runtime.configuration.as_ref().is_some_and(|c| {
            c.reports
                .iter()
                .any(|r| r.source == source && r.recipient == recipient)
        }) || report.units.len() > 1
            || report.facilities.len() > 1
            || report.units.iter().any(|u| {
                u.unit_id != source
                    || u.observed_tick > tick
                    || !u.fuel_kg.is_finite()
                    || u.fuel_kg < 0.0
                    || !u.cargo.valid()
            })
            || report.facilities.iter().any(|f| {
                f.facility.unit_id != source
                    || f.observed_tick > tick
                    || !f.facility.stock.valid()
                    || !f.reserved.valid()
            })
            || report.unavailable.iter().any(|u| u.unit_id != source)
        {
            return false;
        }
        let newest = |p: &Projection| {
            p.units
                .iter()
                .map(|u| u.observed_tick)
                .chain(p.facilities.iter().map(|f| f.observed_tick))
                .max()
                .unwrap_or(0)
        };
        if runtime
            .received
            .get(&(recipient, source))
            .is_some_and(|old| newest(old) >= newest(&report))
        {
            return false;
        }
        for u in &mut report.units {
            u.received_tick = tick;
        }
        for f in &mut report.facilities {
            f.received_tick = tick;
        }
        runtime.received.insert((recipient, source), report);
        true
    }
    pub fn logistics_truth(&self) -> Option<Projection> {
        let r = self.world.resource::<Runtime>();
        let c = r.configuration.as_ref()?;
        let mut p = Projection::default();
        let ids: BTreeSet<_> = r.units.keys().chain(r.facilities.keys()).copied().collect();
        for id in &ids {
            let local = r.local(*id, self.tick());
            p.units.extend(local.units);
            p.facilities.extend(local.facilities);
        }
        p.unavailable = c.unavailable.clone();
        Some(p)
    }
}

/// Returns only the fractional work needed for the next indivisible store. Blocked
/// capacity cannot accumulate work and later bypass physical transfer rates.
fn transfer_stock(from: &mut Stock, to: &mut Stock, capacity: Option<&Stock>, budget: f64) -> f64 {
    let fuel_space = capacity.map_or(f64::MAX, |c| (c.fuel_kg - to.fuel_kg).max(0.0));
    let fuel = from.fuel_kg.min(fuel_space).min(budget);
    from.fuel_kg -= fuel;
    to.fuel_kg += fuel;
    let ammo_space = capacity.map_or(u32::MAX, |c| c.ammunition.saturating_sub(to.ammunition));
    let ammo = from
        .ammunition
        .min(ammo_space)
        .min(((budget - fuel + 1e-9).max(0.0)).floor() as u32);
    from.ammunition -= ammo;
    to.ammunition += ammo;
    let repair_space = capacity.map_or(f64::MAX, |c| (c.repair_points - to.repair_points).max(0.0));
    let repair = from
        .repair_points
        .min(repair_space)
        .min((budget - fuel - f64::from(ammo)).max(0.0));
    from.repair_points -= repair;
    to.repair_points += repair;
    let unused = (budget - fuel - f64::from(ammo) - repair).max(0.0);
    if from.ammunition > 0 && ammo_space > ammo && unused < 1.0 {
        unused
    } else {
        0.0
    }
}
