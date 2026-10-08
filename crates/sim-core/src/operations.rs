//! Small, deterministic campaign/airspace model. All distances are SI and times are ticks.
use super::*;
use sim_geo::{LatLon, MslAltitude, SourceGeometry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub id: Uuid,
    pub description: String,
    pub position: GeoPose,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignPhase {
    pub name: String,
    pub start_tick: u64,
    pub end_tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentTask {
    pub role_id: Uuid,
    pub description: String,
    pub supports_objective: Uuid,
    pub resource_units: Vec<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseOfAction {
    pub id: Uuid,
    pub name: String,
    pub risk_assessment: String,
    pub component_tasks: Vec<ComponentTask>,
    pub missions: Vec<MissionTask>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignPlan {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub intent: String,
    pub commander_role_id: Uuid,
    pub airspace_authority_role_id: Uuid,
    pub objectives: Vec<Objective>,
    pub phases: Vec<CampaignPhase>,
    pub courses: Vec<CourseOfAction>,
    pub selected_course_id: Option<Uuid>,
    pub airspaces: Vec<AirspaceVolume>,
    pub published_tick: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirspaceKind {
    Sector,
    Corridor,
    Restricted,
    Patrol,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlMethod {
    Positive,
    Procedural,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirspaceVolume {
    pub id: Uuid,
    pub name: String,
    pub kind: AirspaceKind,
    pub polygon: Vec<GeoPose>,
    pub floor_m: MslAltitude,
    pub ceiling_m: MslAltitude,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_geometry: Option<SourceGeometry>,
    #[serde(default)]
    pub active_periods: Vec<ActivationPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AirspaceSource>,
    pub start_tick: u64,
    pub end_tick: u64,
    pub controller_role_id: Uuid,
    pub control_method: ControlMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivationPeriod {
    pub start_tick: u64,
    pub end_tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirspaceSource {
    pub order_id: String,
    pub external_id: String,
    pub message_hash: String,
    pub raw: String,
    pub resolutions: serde_json::Value,
}
impl AirspaceVolume {
    pub fn ring(&self) -> Vec<LatLon> {
        self.polygon
            .iter()
            .map(|p| LatLon {
                latitude_deg: p.latitude_deg,
                longitude_deg: p.longitude_deg,
            })
            .collect()
    }
    pub fn periods(&self) -> Vec<ActivationPeriod> {
        if self.active_periods.is_empty() {
            vec![ActivationPeriod {
                start_tick: self.start_tick,
                end_tick: self.end_tick,
            }]
        } else {
            self.active_periods.clone()
        }
    }
    pub fn active(&self, tick: u64) -> bool {
        self.periods()
            .iter()
            .any(|p| p.start_tick <= tick && tick < p.end_tick)
    }
    pub fn covers(&self, start: u64, end: u64) -> bool {
        start < end
            && self
                .periods()
                .iter()
                .any(|p| p.start_tick <= start && end <= p.end_tick)
    }
    pub fn normalize(&mut self) -> Result<(), String> {
        let mut ring = if let Some(g) = &self.source_geometry {
            g.polygon()?
        } else {
            self.ring()
        };
        sim_geo::normalize_ring(&mut ring);
        sim_geo::validate_ring(&ring)?;
        self.polygon = ring
            .into_iter()
            .map(|p| GeoPose {
                latitude_deg: p.latitude_deg,
                longitude_deg: p.longitude_deg,
                altitude_m: 0.0,
            })
            .collect();
        if !self.active_periods.is_empty() {
            self.active_periods.sort_by_key(|p| p.start_tick);
            self.start_tick = self.active_periods[0].start_tick;
            self.end_tick = self.active_periods.last().unwrap().end_tick;
        }
        Ok(())
    }
    pub fn contains(&self, pose: GeoPose, tick: u64) -> bool {
        self.active(tick)
            && pose.altitude_m >= self.floor_m.meters()
            && pose.altitude_m < self.ceiling_m.meters()
            && sim_geo::contains(
                &self.ring(),
                LatLon {
                    latitude_deg: pose.latitude_deg,
                    longitude_deg: pose.longitude_deg,
                },
            )
    }
}

fn crosses(a: GeoPose, b: GeoPose, c: GeoPose, d: GeoPose) -> bool {
    let side = |p: GeoPose, q: GeoPose, r: GeoPose| {
        (q.longitude_deg - p.longitude_deg) * (r.latitude_deg - p.latitude_deg)
            - (q.latitude_deg - p.latitude_deg) * (r.longitude_deg - p.longitude_deg)
    };
    side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
}
fn route_intersects_volume(route: &[GeoPose], volume: &AirspaceVolume) -> bool {
    route.iter().any(|p| volume.contains(*p, volume.start_tick))
        || route.windows(2).any(|pair| {
            pair[0].altitude_m.min(pair[1].altitude_m) < volume.ceiling_m.meters()
                && pair[0].altitude_m.max(pair[1].altitude_m) >= volume.floor_m.meters()
                && sim_geo::segment_intersects(
                    &volume.ring(),
                    LatLon {
                        latitude_deg: pair[0].latitude_deg,
                        longitude_deg: pair[0].longitude_deg,
                    },
                    LatLon {
                        latitude_deg: pair[1].latitude_deg,
                        longitude_deg: pair[1].longitude_deg,
                    },
                )
        })
}
fn routes_conflict(a: &[GeoPose], b: &[GeoPose]) -> bool {
    a.iter().any(|p| {
        b.iter().any(|q| {
            great_circle_distance_m(*p, *q) < 1000.0 && (p.altitude_m - q.altitude_m).abs() < 300.0
        })
    }) || a.windows(2).any(|p| {
        b.windows(2).any(|q| {
            crosses(p[0], p[1], q[0], q[1])
                && p[0].altitude_m.min(p[1].altitude_m)
                    < q[0].altitude_m.max(q[1].altitude_m) + 300.0
                && q[0].altitude_m.min(q[1].altitude_m)
                    < p[0].altitude_m.max(p[1].altitude_m) + 300.0
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionKind {
    Transit,
    Patrol,
    Intercept,
    Strike,
    Return,
    Defend,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LostCommsProcedure {
    Continue,
    Hold,
    Return,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementConstraints {
    pub weapons_release: bool,
    pub minimum_identification: f32,
    pub max_track_age_ticks: u64,
    pub authorized_objective: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionTask {
    pub id: Uuid,
    pub name: String,
    pub kind: MissionKind,
    pub unit_id: Uuid,
    pub issuer_role_id: Uuid,
    pub objective_id: Uuid,
    pub start_tick: u64,
    pub end_tick: u64,
    pub route: Vec<GeoPose>,
    pub speed_mps: f64,
    pub home: GeoPose,
    pub depends_on: Vec<Uuid>,
    pub lost_comms: LostCommsProcedure,
    pub engagement: EngagementConstraints,
    pub clearance_airspace_ids: Vec<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirspaceControlOrder {
    pub plan_id: Uuid,
    pub revision: u64,
    pub volumes: Vec<AirspaceVolume>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirTaskingOrder {
    pub plan_id: Uuid,
    pub revision: u64,
    pub missions: Vec<MissionTask>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clearance {
    pub id: Uuid,
    pub airspace_id: Uuid,
    pub unit_id: Uuid,
    pub controller_role_id: Uuid,
    pub start_tick: u64,
    pub end_tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handoff {
    pub id: Uuid,
    pub unit_id: Uuid,
    pub from_role_id: Uuid,
    pub to_role_id: Uuid,
    pub airspace_id: Uuid,
    pub accepted: bool,
    pub delivered: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseComparison {
    pub id: Uuid,
    pub name: String,
    pub aircraft_required: usize,
    pub mission_ticks: u64,
    pub risk_assessment: String,
}

impl CampaignPlan {
    pub fn comparisons(&self) -> Vec<CourseComparison> {
        self.courses
            .iter()
            .map(|c| CourseComparison {
                id: c.id,
                name: c.name.clone(),
                aircraft_required: c
                    .missions
                    .iter()
                    .map(|m| m.unit_id)
                    .collect::<BTreeSet<_>>()
                    .len(),
                mission_ticks: c
                    .missions
                    .iter()
                    .map(|m| m.end_tick.saturating_sub(m.start_tick))
                    .sum(),
                risk_assessment: c.risk_assessment.clone(),
            })
            .collect()
    }
    pub fn products(&self) -> Result<(AirspaceControlOrder, AirTaskingOrder), String> {
        let course = self
            .courses
            .iter()
            .find(|c| Some(c.id) == self.selected_course_id)
            .ok_or("select a course of action")?;
        Ok((
            AirspaceControlOrder {
                plan_id: self.id,
                revision: self.revision,
                volumes: self.airspaces.clone(),
            },
            AirTaskingOrder {
                plan_id: self.id,
                revision: self.revision,
                missions: course.missions.clone(),
            },
        ))
    }
    pub fn validate(
        &self,
        authority: &AuthorityDefinition,
        units: &BTreeSet<Uuid>,
    ) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.intent.trim().is_empty()
            || self.courses.is_empty()
            || self.objectives.is_empty()
            || self.phases.is_empty()
        {
            return Err("plan needs intent, objectives, phases and courses".into());
        }
        let roles: BTreeMap<_, _> = authority.roles.iter().map(|r| (r.id, r)).collect();
        let commander = roles
            .get(&self.commander_role_id)
            .ok_or("unknown campaign commander")?;
        if !matches!(
            commander.kind,
            AuthorityRoleKind::JointForceCommander | AuthorityRoleKind::CombatantCommander
        ) {
            return Err("campaign commander must hold a joint command appointment".into());
        }
        if roles
            .get(&self.airspace_authority_role_id)
            .is_none_or(|r| r.side != commander.side)
        {
            return Err("invalid airspace authority appointment".into());
        }
        let objective_ids: BTreeSet<_> = self.objectives.iter().map(|o| o.id).collect();
        if objective_ids.len() != self.objectives.len()
            || self
                .objectives
                .iter()
                .any(|o| !geo_pose_is_finite(o.position))
        {
            return Err("invalid or duplicate objectives".into());
        }
        if self.phases.iter().any(|p| p.start_tick >= p.end_tick)
            || self
                .phases
                .windows(2)
                .any(|p| p[0].end_tick > p[1].start_tick)
        {
            return Err("phases must be ordered non-overlapping windows".into());
        }
        let airspace_ids: BTreeSet<_> = self.airspaces.iter().map(|a| a.id).collect();
        if airspace_ids.len() != self.airspaces.len() {
            return Err("duplicate airspace IDs".into());
        }
        for a in &self.airspaces {
            if a.polygon.len() < 3
                || a.polygon.iter().any(|p| !geo_pose_is_finite(*p))
                || !a.floor_m.meters().is_finite()
                || !a.ceiling_m.meters().is_finite()
                || a.floor_m >= a.ceiling_m
                || a.start_tick >= a.end_tick
                || roles
                    .get(&a.controller_role_id)
                    .is_none_or(|r| r.side != commander.side)
            {
                return Err(format!("invalid airspace {}", a.name));
            }
            sim_geo::validate_ring(&a.ring())?;
            let periods = a.periods();
            if periods.iter().any(|p| p.start_tick >= p.end_tick)
                || periods.windows(2).any(|p| p[0].end_tick > p[1].start_tick)
            {
                return Err(
                    "airspace activation periods must be ordered, non-overlapping windows".into(),
                );
            }
        }
        for (i, a) in self.airspaces.iter().enumerate() {
            for b in &self.airspaces[i + 1..] {
                if a.kind == AirspaceKind::Sector
                    && b.kind == AirspaceKind::Sector
                    && a.controller_role_id != b.controller_role_id
                    && overlap(a, b)
                {
                    return Err("overlapping sector controller assignments".into());
                }
            }
        }
        let mut courses = BTreeSet::new();
        for c in &self.courses {
            if !courses.insert(c.id) {
                return Err("duplicate course IDs".into());
            }
            for task in &c.component_tasks {
                if roles
                    .get(&task.role_id)
                    .is_none_or(|r| r.side != commander.side)
                    || !objective_ids.contains(&task.supports_objective)
                    || task
                        .resource_units
                        .iter()
                        .any(|u| !authority.role_is_in_unit_chain(task.role_id, *u))
                {
                    return Err("invalid component allocation".into());
                }
            }
            let mut previous = BTreeMap::new();
            for (i, m) in c.missions.iter().enumerate() {
                if previous.contains_key(&m.id)
                    || m.start_tick >= m.end_tick
                    || m.route.is_empty()
                    || m.route
                        .iter()
                        .any(|p| !geo_pose_is_finite(*p) || p.altitude_m < 0.0)
                    || !geo_pose_is_finite(m.home)
                    || !m.speed_mps.is_finite()
                    || !(1.0..=700.0).contains(&m.speed_mps)
                    || !units.contains(&m.unit_id)
                    || !objective_ids.contains(&m.objective_id)
                {
                    return Err(format!("invalid mission {}", m.name));
                }
                if !authority.role_is_in_unit_chain(self.commander_role_id, m.unit_id)
                    || !authority.role_is_in_unit_chain(m.issuer_role_id, m.unit_id)
                {
                    return Err("mission exceeds command authority".into());
                }
                if !self
                    .phases
                    .iter()
                    .any(|p| p.start_tick <= m.start_tick && p.end_tick >= m.end_tick)
                {
                    return Err("mission must fit in a campaign phase".into());
                }
                if m.depends_on
                    .iter()
                    .any(|id| previous.get(id).is_none_or(|end| *end > m.start_tick))
                {
                    return Err(
                        "dependencies must precede the mission in time and task order".into(),
                    );
                }
                if m.clearance_airspace_ids
                    .iter()
                    .any(|id| !airspace_ids.contains(id))
                {
                    return Err("unknown clearance airspace".into());
                }
                if !m.engagement.minimum_identification.is_finite()
                    || !(0.8..=1.0).contains(&m.engagement.minimum_identification)
                    || m.engagement.max_track_age_ticks == 0
                    || (m.engagement.weapons_release
                        && m.engagement.authorized_objective != Some(m.objective_id))
                {
                    return Err("invalid engagement constraints".into());
                }
                if matches!(m.kind, MissionKind::Strike | MissionKind::Intercept)
                    && m.engagement.weapons_release
                    && authority
                        .policy_for(ACTION_ENGAGE, m.unit_id)
                        .is_none_or(|p| {
                            !p.direct_role_ids.contains(&self.commander_role_id) || !p.executable
                        })
                {
                    return Err("commander lacks weapons-release grant".into());
                }
                for other in &c.missions[..i] {
                    if m.start_tick < other.end_tick && other.start_tick < m.end_tick {
                        if m.unit_id == other.unit_id {
                            return Err("aircraft allocated to overlapping missions".into());
                        }
                        if routes_conflict(&m.route, &other.route) {
                            return Err("mission routes conflict in time and altitude".into());
                        }
                    }
                }
                if self.airspaces.iter().any(|a| {
                    a.kind == AirspaceKind::Restricted
                        && a.start_tick < m.end_tick
                        && m.start_tick < a.end_tick
                        && route_intersects_volume(&m.route, a)
                        && !m.clearance_airspace_ids.contains(&a.id)
                }) {
                    return Err("route enters restricted airspace without clearance".into());
                }
                previous.insert(m.id, m.end_tick);
            }
        }
        if self
            .selected_course_id
            .is_some_and(|id| !courses.contains(&id))
        {
            return Err("unknown selected course".into());
        }
        Ok(())
    }
}

fn overlap(a: &AirspaceVolume, b: &AirspaceVolume) -> bool {
    a.periods().iter().any(|a| {
        b.periods()
            .iter()
            .any(|b| a.start_tick < b.end_tick && b.start_tick < a.end_tick)
    }) && a.floor_m < b.ceiling_m
        && b.floor_m < a.ceiling_m
        && sim_geo::polygons_overlap(&a.ring(), &b.ring())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    Scheduled,
    Active,
    Holding,
    Returning,
    Completed,
    Failed,
    Cancelled,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionReport {
    pub mission_id: Uuid,
    pub unit_id: Uuid,
    pub state: MissionState,
    pub observed_tick: u64,
    pub detail: String,
    pub fuel_seconds: f64,
    pub ammunition: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatProfile {
    pub fuel_seconds: f64,
    pub ammunition: u32,
    pub weapon_range_m: f64,
    pub weapon_speed_mps: f64,
    pub hit_probability_bps: u16,
    pub source: String,
}
impl Default for CombatProfile {
    fn default() -> Self {
        Self {
            fuel_seconds: 3600.0,
            ammunition: 2,
            weapon_range_m: 40_000.0,
            weapon_speed_mps: 800.0,
            hit_probability_bps: 6500,
            source:
                "Gameplay estimate for generic training aircraft; not a real platform specification"
                    .into(),
        }
    }
}
#[derive(Component, Debug, Clone)]
pub struct CombatState {
    pub profile: CombatProfile,
    pub destroyed: bool,
}
#[derive(Clone)]
struct Execution {
    plan_id: Uuid,
    task: MissionTask,
    revision: u64,
    waypoint: usize,
    state: MissionState,
    controller: Option<Uuid>,
    last_contact: u64,
    fired: bool,
    detail: String,
}
struct Weapon {
    target: Uuid,
    impact_tick: u64,
    hit: bool,
}
#[derive(Resource, Default)]
pub(super) struct Operations {
    missions: BTreeMap<Uuid, Execution>,
    airspaces: BTreeMap<Uuid, Vec<AirspaceVolume>>,
    clearances: BTreeMap<Uuid, Vec<Clearance>>,
    weapons: Vec<Weapon>,
    pub(super) iftu_units: BTreeSet<Uuid>,
    pub(super) launches: Vec<crate::iftu::Launch>,
    pub seed: u64,
    known_completions: BTreeMap<Uuid, BTreeSet<Uuid>>,
    revisions: BTreeMap<(Uuid, Uuid), u64>,
}
impl Operations {
    pub(super) fn truth_weapons(&self) -> Vec<crate::truth::TruthWeapon> {
        self.weapons
            .iter()
            .enumerate()
            .map(|(i, w)| crate::truth::TruthWeapon {
                id: format!("campaign-impact-{}-{}-{i}", w.target, w.impact_tick),
                kind: "campaign_pending_impact".into(),
                side: None,
                launcher_id: None,
                position: None,
                aim_point: None,
                impact_tick: Some(w.impact_tick),
                phase: "pending impact".into(),
                provider_id: None,
            })
            .collect()
    }

    pub(super) fn new(seed: u64) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }
    pub(super) fn tactical_move(&mut self, unit: Uuid) {
        for m in self.missions.values_mut().filter(|m| {
            m.task.unit_id == unit
                && matches!(
                    m.state,
                    MissionState::Active | MissionState::Holding | MissionState::Returning
                )
        }) {
            m.state = MissionState::Cancelled;
            m.detail = "Superseded by received tactical movement order".into();
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn engage(
        &mut self,
        unit: Uuid,
        pose: GeoPose,
        combat: &mut CombatState,
        track: &Track,
        target: Uuid,
        tick: u64,
    ) -> Result<(), String> {
        let m = self
            .missions
            .values()
            .find(|m| {
                m.task.unit_id == unit
                    && m.state == MissionState::Active
                    && m.task.engagement.weapons_release
            })
            .ok_or("no active mission weapons-release constraints")?;
        if combat.destroyed
            || combat.profile.ammunition == 0
            || tick.saturating_sub(track.observed_tick) > m.task.engagement.max_track_age_ticks
            || track.identity_confidence < m.task.engagement.minimum_identification
            || track.assessed_destroyed == Some(true)
        {
            return Err("engagement prerequisites not met".into());
        }
        if great_circle_distance_m(track.position, *m.task.route.last().unwrap()) > 30_000.0 {
            return Err("track outside authorized objective area".into());
        }
        let distance = great_circle_distance_m(pose, track.position)
            .hypot(pose.altitude_m - track.position.altitude_m);
        if distance > combat.profile.weapon_range_m || !line_of_sight(pose, track.position) {
            return Err("track outside weapon envelope".into());
        }
        let sample = (self.seed ^ unit.as_u128() as u64 ^ tick.wrapping_mul(0x9e3779b97f4a7c15))
            .wrapping_mul(0xbf58476d1ce4e5b9);
        if self.iftu_units.contains(&unit) {
            self.launches.push(crate::iftu::Launch {
                launcher: unit,
                position: pose,
                side: if track.target_side == Some(Side::Red) {
                    Side::Blue
                } else {
                    Side::Red
                },
                track: track.clone(),
            });
        } else {
            self.weapons.push(Weapon {
                target,
                impact_tick: tick
                    + (distance / combat.profile.weapon_speed_mps).ceil().max(1.0) as u64,
                hit: sample % 10000 < u64::from(combat.profile.hit_probability_bps),
            });
        }
        combat.profile.ammunition -= 1;
        Ok(())
    }
}

impl Simulation {
    pub fn receive_mission_report(&mut self, unit: Uuid, report: &MissionReport) {
        if report.state == MissionState::Completed {
            self.world
                .resource_mut::<Operations>()
                .known_completions
                .entry(unit)
                .or_default()
                .insert(report.mission_id);
        }
    }
    pub fn receive_controller_heartbeat(&mut self, unit: Uuid, controller: Uuid) {
        let tick = self.tick();
        let mut ops = self.world.resource_mut::<Operations>();
        let assigned = ops
            .airspaces
            .get(&unit)
            .is_some_and(|volumes| volumes.iter().any(|v| v.controller_role_id == controller));
        for m in ops.missions.values_mut().filter(|m| {
            m.task.unit_id == unit
                && (m.controller == Some(controller) || (m.controller.is_none() && assigned))
        }) {
            m.last_contact = tick;
        }
    }
    pub fn receive_tasking(
        &mut self,
        unit: Uuid,
        aco: &AirspaceControlOrder,
        ato: &AirTaskingOrder,
    ) {
        let tick = self.tick();
        let mut tasking_changed = false;
        {
            let mut ops = self.world.resource_mut::<Operations>();
            if ops
                .revisions
                .get(&(unit, ato.plan_id))
                .is_some_and(|revision| *revision >= ato.revision)
            {
                return;
            }
            ops.revisions.insert((unit, ato.plan_id), ato.revision);
            let changed: BTreeSet<_> = ops
                .airspaces
                .get(&unit)
                .into_iter()
                .flatten()
                .filter(|old| {
                    aco.volumes.iter().find(|v| v.id == old.id).is_none_or(|v| {
                        serde_json::to_value(v).ok() != serde_json::to_value(old).ok()
                    })
                })
                .map(|v| v.id)
                .collect();
            if let Some(clearances) = ops.clearances.get_mut(&unit) {
                clearances.retain(|c| !changed.contains(&c.airspace_id));
            }
            ops.airspaces.insert(unit, aco.volumes.clone());
            for old in ops.missions.values_mut().filter(|m| {
                m.task.unit_id == unit
                    && m.plan_id == ato.plan_id
                    && !ato.missions.iter().any(|task| task.id == m.task.id)
            }) {
                tasking_changed = true;
                old.state = MissionState::Cancelled;
                old.detail = "Superseded by received tasking revision".into();
            }
            for task in ato.missions.iter().filter(|m| m.unit_id == unit) {
                if ops
                    .missions
                    .get(&task.id)
                    .is_some_and(|m| m.revision >= ato.revision)
                {
                    continue;
                }
                if let Some(old) = ops.missions.get_mut(&task.id) {
                    if serde_json::to_value(&old.task).ok() == serde_json::to_value(task).ok() {
                        old.revision = ato.revision;
                        continue;
                    }
                }
                tasking_changed = true;
                let fired = ops.missions.get(&task.id).is_some_and(|m| m.fired);
                ops.missions.insert(
                    task.id,
                    Execution {
                        plan_id: ato.plan_id,
                        task: task.clone(),
                        revision: ato.revision,
                        waypoint: 0,
                        state: MissionState::Scheduled,
                        controller: None,
                        last_contact: tick,
                        fired,
                        detail: "Tasking received".into(),
                    },
                );
            }
            for volume in aco
                .volumes
                .iter()
                .filter(|a| a.control_method == ControlMethod::Procedural)
            {
                for task in ato
                    .missions
                    .iter()
                    .filter(|m| m.unit_id == unit && m.clearance_airspace_ids.contains(&volume.id))
                {
                    ops.clearances.entry(unit).or_default().push(Clearance {
                        id: task.id,
                        airspace_id: volume.id,
                        unit_id: unit,
                        controller_role_id: volume.controller_role_id,
                        start_tick: task.start_tick,
                        end_tick: task.end_tick,
                    });
                }
            }
        }
        if !tasking_changed {
            return;
        }
        let mut query = self.world.query::<(
            &SimEntityId,
            &mut Velocity,
            Option<&mut CyclicFlightPathState>,
        )>();
        for (id, mut velocity, path) in query.iter_mut(&mut self.world) {
            if id.0 == unit {
                *velocity = Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                };
                if let Some(mut path) = path {
                    path.active = false;
                }
            }
        }
    }
    pub fn receive_clearance(&mut self, clearance: Clearance) {
        let tick = self.tick();
        let mut ops = self.world.resource_mut::<Operations>();
        for execution in ops
            .missions
            .values_mut()
            .filter(|m| m.task.unit_id == clearance.unit_id)
        {
            execution.controller = Some(clearance.controller_role_id);
            execution.last_contact = tick;
        }
        let clearances = ops.clearances.entry(clearance.unit_id).or_default();
        clearances.retain(|old| old.id != clearance.id);
        clearances.push(clearance);
    }
    pub fn cancel_missions(&mut self, unit: Uuid, ids: &[Uuid]) {
        for m in self
            .world
            .resource_mut::<Operations>()
            .missions
            .values_mut()
            .filter(|m| m.task.unit_id == unit && ids.contains(&m.task.id))
        {
            m.state = MissionState::Cancelled;
            m.detail = "Cancellation received".into();
        }
        let mut query = self.world.query::<(&SimEntityId, &mut Velocity)>();
        for (id, mut velocity) in query.iter_mut(&mut self.world) {
            if id.0 == unit {
                *velocity = Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                };
            }
        }
    }
    pub fn mission_reports(&mut self, unit: Uuid) -> Vec<MissionReport> {
        let tick = self.tick();
        let mut query = self.world.query::<(&SimEntityId, &CombatState)>();
        let profile = query
            .iter(&self.world)
            .find(|(id, _)| id.0 == unit)
            .map(|(_, c)| c.profile.clone())
            .unwrap_or_default();
        if query
            .iter(&self.world)
            .any(|(id, c)| id.0 == unit && c.destroyed)
        {
            return Vec::new();
        }
        self.world
            .resource::<Operations>()
            .missions
            .values()
            .filter(|m| m.task.unit_id == unit)
            .map(|m| MissionReport {
                mission_id: m.task.id,
                unit_id: unit,
                state: m.state,
                observed_tick: tick,
                detail: m.detail.clone(),
                fuel_seconds: profile.fuel_seconds,
                ammunition: profile.ammunition,
            })
            .collect()
    }
    pub fn set_combat_profile(&mut self, unit: Uuid, profile: CombatProfile) -> Result<(), String> {
        if !profile.fuel_seconds.is_finite()
            || profile.fuel_seconds < 0.0
            || !profile.weapon_range_m.is_finite()
            || profile.weapon_range_m <= 0.0
            || !profile.weapon_speed_mps.is_finite()
            || profile.weapon_speed_mps <= 0.0
            || profile.hit_probability_bps > 10000
            || profile.source.is_empty()
        {
            return Err("invalid combat profile".into());
        }
        let mut query = self.world.query::<(&SimEntityId, &mut CombatState)>();
        let (_, mut state) = query
            .iter_mut(&mut self.world)
            .find(|(id, _)| id.0 == unit)
            .ok_or("unknown unit")?;
        state.profile = profile;
        Ok(())
    }
}

pub(super) fn advance_operations(world: &mut World) {
    let tick = world.resource::<SimClock>().tick;
    let mut ops = world.remove_resource::<Operations>().unwrap();
    let mut query = world.query::<(&SimEntityId, &GeoPose, &Ownership, &CombatState)>();
    let states: BTreeMap<_, _> = query
        .iter(world)
        .map(|(id, pose, side, combat)| (id.0, (*pose, side.0, combat.clone())))
        .collect();
    let mut hits = BTreeSet::new();
    ops.weapons.retain(|w| {
        if tick >= w.impact_tick {
            if w.hit {
                hits.insert(w.target);
            }
            false
        } else {
            true
        }
    });
    let knowledge = world.resource::<KnowledgeBases>().0.clone();
    let targets = world.resource::<ReportKnowledge>().targets.clone();
    let completed: BTreeSet<_> = ops
        .missions
        .values()
        .filter(|m| m.state == MissionState::Completed)
        .map(|m| (m.task.unit_id, m.task.id))
        .collect();
    let mut motion = BTreeMap::new();
    let mut shots = BTreeSet::new();
    for m in ops.missions.values_mut() {
        let task = &m.task;
        if tick < task.start_tick
            || matches!(
                m.state,
                MissionState::Completed | MissionState::Cancelled | MissionState::Failed
            )
        {
            continue;
        }
        let Some((pose, side, combat)) = states.get(&task.unit_id) else {
            continue;
        };
        if combat.destroyed || hits.contains(&task.unit_id) || combat.profile.fuel_seconds <= 0.0 {
            m.state = MissionState::Failed;
            m.detail = "Aircraft unavailable".into();
            motion.insert(
                task.unit_id,
                Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                },
            );
            continue;
        }
        if tick >= task.end_tick && m.state != MissionState::Returning {
            m.state = MissionState::Returning;
            m.detail = "Task window ended; returning".into();
        }
        if m.state != MissionState::Returning
            && !task.depends_on.iter().all(|id| {
                completed.contains(&(task.unit_id, *id))
                    || ops
                        .known_completions
                        .get(&task.unit_id)
                        .is_some_and(|known| known.contains(id))
            })
        {
            m.state = MissionState::Holding;
            m.detail = "Waiting for locally known dependency completion".into();
            motion.insert(
                task.unit_id,
                Velocity {
                    north_mps: 0.0,
                    east_mps: 0.0,
                    climb_mps: 0.0,
                },
            );
            continue;
        }
        if m.state == MissionState::Scheduled {
            m.state = MissionState::Active;
        }
        // A fresh report from the assigned controller's node is the communications heartbeat.
        let disconnected = tick.saturating_sub(m.last_contact) > 30;
        if disconnected && m.state != MissionState::Returning {
            match task.lost_comms {
                LostCommsProcedure::Return => {
                    m.state = MissionState::Returning;
                    m.detail = "Lost communications: return".into();
                }
                LostCommsProcedure::Hold => {
                    m.state = MissionState::Holding;
                    m.detail = "Lost communications: hold".into();
                }
                LostCommsProcedure::Continue => {}
            }
        } else if m.state == MissionState::Holding {
            m.state = MissionState::Active;
        }
        if let Some(volumes) = ops.airspaces.get(&task.unit_id) {
            for volume in volumes.iter().filter(|a| a.contains(*pose, tick)) {
                let cleared = ops.clearances.get(&task.unit_id).is_some_and(|cs| {
                    cs.iter().any(|c| {
                        c.airspace_id == volume.id && c.start_tick <= tick && tick < c.end_tick
                    })
                });
                if !cleared {
                    m.detail = format!("Airspace violation: {}", volume.name);
                }
            }
        }
        let mut destination = if m.state == MissionState::Returning {
            task.home
        } else {
            task.route[m.waypoint.min(task.route.len() - 1)]
        };
        if matches!(
            task.kind,
            MissionKind::Intercept
                | MissionKind::Strike
                | MissionKind::Defend
                | MissionKind::Patrol
        ) && m.state == MissionState::Active
            && task.engagement.weapons_release
            && !m.fired
            && combat.profile.ammunition > 0
        {
            if let Some((target, track)) = knowledge
                .get(&task.unit_id)
                .into_iter()
                .flatten()
                .filter(|t| {
                    t.target_side.is_some_and(|s| s != *side)
                        && t.assessed_destroyed != Some(true)
                        && t.observed_domain
                            == Some(if task.kind == MissionKind::Strike {
                                Domain::Land
                            } else {
                                Domain::Air
                            })
                        && t.identity_confidence >= task.engagement.minimum_identification
                        && tick.saturating_sub(t.observed_tick)
                            <= task.engagement.max_track_age_ticks
                })
                .filter_map(|t| {
                    if ops.iftu_units.contains(&task.unit_id) {
                        return Some((Uuid::nil(), t));
                    }
                    targets
                        .iter()
                        .find(|((owner, _), id)| *owner == task.unit_id && **id == t.track_id)
                        .map(|((_, target), _)| (*target, t))
                })
                .find(|(_, t)| {
                    great_circle_distance_m(t.position, *task.route.last().unwrap()) <= 30_000.0
                })
            {
                if task.kind == MissionKind::Intercept {
                    destination = track.position;
                }
                let distance = great_circle_distance_m(*pose, track.position)
                    .hypot(pose.altitude_m - track.position.altitude_m);
                if distance <= combat.profile.weapon_range_m && line_of_sight(*pose, track.position)
                {
                    let mut sample =
                        ops.seed ^ task.id.as_u128() as u64 ^ tick.wrapping_mul(0x9e3779b97f4a7c15);
                    sample ^= sample >> 30;
                    sample = sample.wrapping_mul(0xbf58476d1ce4e5b9);
                    sample ^= sample >> 27;
                    if ops.iftu_units.contains(&task.unit_id) {
                        ops.launches.push(crate::iftu::Launch {
                            launcher: task.unit_id,
                            position: *pose,
                            side: if track.target_side == Some(Side::Red) {
                                Side::Blue
                            } else {
                                Side::Red
                            },
                            track: track.clone(),
                        });
                    } else {
                        ops.weapons.push(Weapon {
                            target,
                            impact_tick: tick
                                + (distance / combat.profile.weapon_speed_mps).ceil().max(1.0)
                                    as u64,
                            hit: sample % 10000 < u64::from(combat.profile.hit_probability_bps),
                        });
                    }
                    shots.insert(task.unit_id);
                    m.fired = true;
                    m.detail = "Weapon released; effect unconfirmed".into();
                }
            }
        }
        let distance = great_circle_distance_m(*pose, destination);
        if task.kind != MissionKind::Defend
            && distance <= task.speed_mps
            && (pose.altitude_m - destination.altitude_m).abs() < 100.0
        {
            if m.state == MissionState::Returning || task.kind == MissionKind::Return {
                m.state = MissionState::Completed;
                m.detail = if m.fired {
                    "Returned; combat effect requires assessment".into()
                } else {
                    "Returned".into()
                };
            } else if m.waypoint + 1 < task.route.len() {
                m.waypoint += 1;
            } else if task.kind == MissionKind::Patrol {
                m.waypoint = 0;
            } else {
                m.state = MissionState::Returning;
            }
        }
        let velocity = if task.kind == MissionKind::Defend
            || matches!(m.state, MissionState::Holding | MissionState::Completed)
        {
            Velocity {
                north_mps: 0.0,
                east_mps: 0.0,
                climb_mps: 0.0,
            }
        } else {
            let north = (destination.latitude_deg - pose.latitude_deg) * 111_320.0;
            let east = (destination.longitude_deg - pose.longitude_deg)
                * 111_320.0
                * pose.latitude_deg.to_radians().cos();
            let norm = north.hypot(east).max(task.speed_mps);
            Velocity {
                north_mps: north / norm * task.speed_mps,
                east_mps: east / norm * task.speed_mps,
                climb_mps: (destination.altitude_m - pose.altitude_m).clamp(-50.0, 50.0),
            }
        };
        motion.insert(task.unit_id, velocity);
    }
    let mut query = world.query::<(
        &SimEntityId,
        &mut Velocity,
        &mut CombatState,
        Option<&mut CyclicFlightPathState>,
    )>();
    for (id, mut velocity, mut combat, mut path) in query.iter_mut(world) {
        if hits.contains(&id.0) {
            combat.destroyed = true;
        }
        if let Some(next) = motion.get(&id.0) {
            *velocity = *next;
            if let Some(p) = path.as_deref_mut() {
                p.active = false;
            }
            combat.profile.fuel_seconds = (combat.profile.fuel_seconds - 1.0).max(0.0);
        }
        if shots.contains(&id.0) {
            combat.profile.ammunition = combat.profile.ammunition.saturating_sub(1);
        }
        if combat.destroyed {
            if let Some(p) = path.as_deref_mut() {
                p.active = false;
            }
            *velocity = Velocity {
                north_mps: 0.0,
                east_mps: 0.0,
                climb_mps: 0.0,
            };
        }
    }
    world.insert_resource(ops);
}
