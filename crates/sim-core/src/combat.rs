//! Fictional point-directed training weapons. No seeker, guidance, or real weapon data.
use super::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingWeapon {
    pub ammunition: u32,
    pub range_m: f64,
    pub speed_mps: f64,
    pub blast_radius_m: f64,
    pub damage: u32,
    pub max_track_age_ticks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatUnitDefinition {
    pub unit_id: Uuid,
    pub hit_points: u32,
    pub weapon: Option<TrainingWeapon>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainingMission {
    pub title: String,
    pub side: Side,
    pub target_unit_ids: Vec<Uuid>,
    pub deadline_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatConfig {
    pub units: Vec<CombatUnitDefinition>,
    pub mission: Option<TrainingMission>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Active,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionOutcome {
    pub title: String,
    pub status: MissionStatus,
    pub deadline_tick: u64,
    pub finished_tick: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeaponStatus {
    pub ammunition: u32,
    pub range_m: f64,
    pub max_track_age_ticks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactReport {
    pub intent_id: Uuid,
    pub observed_tick: u64,
    pub launched_tick: u64,
    pub resolved_tick: u64,
    pub hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceivedImpactReport {
    pub report: ImpactReport,
    pub received_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatProjection {
    pub mission: Option<MissionOutcome>,
    pub local_shots_in_flight: usize,
    pub local_impacts: Vec<ImpactReport>,
    pub received_impacts: Vec<ReceivedImpactReport>,
}

struct Flight {
    intent_id: Uuid,
    attacker: Uuid,
    side: Side,
    position: GeoPose,
    impact_tick: u64,
    observed_tick: u64,
    launched_tick: u64,
    blast_radius_m: f64,
    damage: u32,
}

#[derive(Resource, Default)]
pub(super) struct CombatState {
    health: BTreeMap<Uuid, u32>,
    sides: BTreeMap<Uuid, Side>,
    weapons: BTreeMap<Uuid, TrainingWeapon>,
    designations: BTreeMap<Uuid, Track>,
    flights: Vec<Flight>,
    reports: Vec<(Uuid, ImpactReport)>,
    received: BTreeMap<Uuid, Vec<ReceivedImpactReport>>,
    mission: Option<TrainingMission>,
    outcome: Option<MissionOutcome>,
}

impl CombatConfig {
    pub fn validate(&self, units: &BTreeMap<Uuid, Side>) -> Result<(), String> {
        let mut configured = BTreeSet::new();
        for unit in &self.units {
            if !units.contains_key(&unit.unit_id)
                || !configured.insert(unit.unit_id)
                || unit.hit_points == 0
            {
                return Err(
                    "combat units must be unique existing platforms with positive hit points"
                        .into(),
                );
            }
            if let Some(weapon) = &unit.weapon {
                if weapon.ammunition == 0
                    || weapon.damage == 0
                    || weapon.max_track_age_ticks == 0
                    || !weapon.range_m.is_finite()
                    || weapon.range_m <= 0.0
                    || !weapon.speed_mps.is_finite()
                    || weapon.speed_mps <= 0.0
                    || !weapon.blast_radius_m.is_finite()
                    || weapon.blast_radius_m <= 0.0
                    || weapon.blast_radius_m > weapon.range_m
                    || !(weapon.range_m / weapon.speed_mps).is_finite()
                    || weapon.range_m / weapon.speed_mps > 86_400.0
                {
                    return Err("training weapons need finite positive limits, ammunition, damage, and a flight envelope of at most one day".into());
                }
            }
        }
        if let Some(mission) = &self.mission {
            let targets: BTreeSet<_> = mission.target_unit_ids.iter().copied().collect();
            if mission.title.trim().is_empty()
                || mission.deadline_tick == 0
                || targets.is_empty()
                || targets.len() != mission.target_unit_ids.len()
                || targets
                    .iter()
                    .any(|id| !configured.contains(id) || units.get(id) == Some(&mission.side))
                || !self.units.iter().any(|unit| {
                    unit.weapon.is_some() && units.get(&unit.unit_id) == Some(&mission.side)
                })
            {
                return Err("training missions need unique configured enemy targets, an armed friendly platform, a title, and a positive deadline".into());
            }
        }
        Ok(())
    }
}

impl CombatState {
    pub(super) fn from_config(config: CombatConfig, sides: BTreeMap<Uuid, Side>) -> Self {
        let outcome = config.mission.as_ref().map(|mission| MissionOutcome {
            title: mission.title.clone(),
            status: MissionStatus::Active,
            deadline_tick: mission.deadline_tick,
            finished_tick: None,
            reason: None,
        });
        Self {
            health: config
                .units
                .iter()
                .map(|unit| (unit.unit_id, unit.hit_points))
                .collect(),
            weapons: config
                .units
                .into_iter()
                .filter_map(|unit| unit.weapon.map(|weapon| (unit.unit_id, weapon)))
                .collect(),
            sides,
            mission: config.mission,
            outcome,
            ..Default::default()
        }
    }

    pub(super) fn weapon_status(&self, unit: Uuid) -> Option<WeaponStatus> {
        self.weapons.get(&unit).map(|weapon| WeaponStatus {
            ammunition: weapon.ammunition,
            range_m: weapon.range_m,
            max_track_age_ticks: weapon.max_track_age_ticks,
        })
    }

    pub(super) fn hit_points(&self, unit: Uuid) -> Option<u32> {
        self.health.get(&unit).copied()
    }

    pub(super) fn projection(&self, terminal: Uuid, side: Side) -> Option<CombatProjection> {
        if self.health.is_empty() {
            return None;
        }
        Some(CombatProjection {
            mission: self
                .mission
                .as_ref()
                .filter(|mission| mission.side == side)
                .and(self.outcome.clone()),
            local_shots_in_flight: self
                .flights
                .iter()
                .filter(|flight| flight.attacker == terminal)
                .count(),
            received_impacts: self.received.get(&terminal).cloned().unwrap_or_default(),
            local_impacts: self
                .reports
                .iter()
                .filter(|(attacker, _)| *attacker == terminal)
                .map(|(_, report)| report.clone())
                .collect(),
        })
    }

    pub(super) fn local_reports(&self, terminal: Uuid) -> Vec<ImpactReport> {
        self.reports
            .iter()
            .filter(|(source, _)| *source == terminal)
            .map(|(_, report)| report.clone())
            .collect()
    }

    pub(super) fn receive(&mut self, terminal: Uuid, report: ImpactReport, received_tick: u64) {
        let reports = self.received.entry(terminal).or_default();
        if !reports
            .iter()
            .any(|known| known.report.intent_id == report.intent_id)
        {
            reports.push(ReceivedImpactReport {
                report,
                received_tick,
            });
        }
    }

    pub(super) fn complete(&self) -> bool {
        self.outcome
            .as_ref()
            .is_some_and(|outcome| outcome.status != MissionStatus::Active)
    }

    pub(super) fn designation(&self, id: Uuid) -> Option<&Track> {
        self.designations.get(&id)
    }

    pub(super) fn designate(
        &mut self,
        id: Uuid,
        attacker: Uuid,
        side: Side,
        track: Track,
        tick: u64,
    ) -> Result<(), String> {
        let weapon = self
            .weapons
            .get(&attacker)
            .ok_or("this platform has no training weapon")?;
        check_track(&track, side, tick, weapon.max_track_age_ticks)?;
        self.designations.insert(id, track);
        Ok(())
    }

    pub(super) fn launch(
        &mut self,
        intent: &PlayerIntent,
        side: Side,
        position: GeoPose,
        tick: u64,
    ) -> Result<(), String> {
        if self.complete() {
            return Err("the training mission has ended".into());
        }
        let track = self
            .designations
            .remove(&intent.intent_id)
            .ok_or("no scoped target report was designated for this order")?;
        let weapon = self
            .weapons
            .get_mut(&intent.target)
            .ok_or("this platform has no training weapon")?;
        check_track(&track, side, tick, weapon.max_track_age_ticks)?;
        if weapon.ammunition == 0 {
            return Err("training ammunition exhausted".into());
        }
        let distance = geodesy::slant_distance_m(position, track.position);
        if distance > weapon.range_m
            || !geodesy::has_geometric_line_of_sight(position, track.position)
        {
            return Err(
                "reported aim point is outside the weapon range or geometric line of sight".into(),
            );
        }
        let travel_ticks = (distance / weapon.speed_mps).ceil().max(1.0) as u64;
        let impact_tick = tick
            .checked_add(travel_ticks)
            .ok_or("weapon flight exceeds the simulation clock")?;
        weapon.ammunition -= 1;
        self.flights.push(Flight {
            intent_id: intent.intent_id,
            attacker: intent.target,
            side,
            position: track.position,
            impact_tick,
            observed_tick: track.observed_tick,
            launched_tick: tick,
            blast_radius_m: weapon.blast_radius_m,
            damage: weapon.damage,
        });
        Ok(())
    }
}

fn check_track(track: &Track, side: Side, tick: u64, max_age: u64) -> Result<(), String> {
    if track.target_side == Some(side)
        || !track.identity_confidence.is_finite()
        || !(0.8..=1.0).contains(&track.identity_confidence)
    {
        return Err("engagement requires an identified hostile contact".into());
    }
    if track.observed_tick > tick || tick - track.observed_tick > max_age {
        return Err("the target report is too old to engage".into());
    }
    Ok(())
}

pub(super) fn resolve_impacts(world: &mut World) {
    let tick = world.resource::<SimClock>().tick;
    let mut combat = world
        .remove_resource::<CombatState>()
        .expect("combat resource exists");
    if combat.health.is_empty() {
        world.insert_resource(combat);
        return;
    }
    let mut query = world.query::<(Entity, &SimEntityId, &Ownership, &GeoPose)>();
    let targets: BTreeMap<_, _> = query
        .iter(world)
        .map(|(entity, id, side, pose)| (id.0, (entity, side.0, *pose)))
        .collect();
    let mut remaining = Vec::new();
    let mut destroyed = BTreeSet::new();
    for flight in std::mem::take(&mut combat.flights) {
        if flight.impact_tick > tick {
            remaining.push(flight);
            continue;
        }
        let mut hit = false;
        for (id, (_, side, pose)) in &targets {
            let Some(health) = combat.health.get_mut(id) else {
                continue;
            };
            if *health > 0
                && *side != flight.side
                && geodesy::slant_distance_m(flight.position, *pose) <= flight.blast_radius_m
                && geodesy::has_geometric_line_of_sight(flight.position, *pose)
            {
                *health = health.saturating_sub(flight.damage);
                hit = true;
                if *health == 0 {
                    destroyed.insert(*id);
                }
            }
        }
        combat.reports.push((
            flight.attacker,
            ImpactReport {
                intent_id: flight.intent_id,
                observed_tick: flight.observed_tick,
                launched_tick: flight.launched_tick,
                resolved_tick: tick,
                hit,
            },
        ));
    }
    combat.flights = remaining;
    for id in destroyed {
        if let Some((entity, _, _)) = targets.get(&id) {
            world.despawn(*entity);
        }
    }
    if let (Some(mission), Some(outcome)) = (&combat.mission, &mut combat.outcome) {
        if outcome.status == MissionStatus::Active {
            let succeeded = mission
                .target_unit_ids
                .iter()
                .all(|id| combat.health.get(id) == Some(&0));
            let exhausted = combat
                .flights
                .iter()
                .all(|flight| flight.side != mission.side)
                && combat
                    .weapons
                    .iter()
                    .filter(|(id, _)| combat.sides.get(id) == Some(&mission.side))
                    .all(|(id, weapon)| {
                        weapon.ammunition == 0 || combat.health.get(id) == Some(&0)
                    });
            if succeeded || tick >= mission.deadline_tick || exhausted {
                outcome.status = if succeeded {
                    MissionStatus::Succeeded
                } else {
                    MissionStatus::Failed
                };
                outcome.finished_tick = Some(tick);
                outcome.reason = Some(
                    if succeeded {
                        "Training target destroyed"
                    } else if exhausted {
                        "No training shots remain"
                    } else {
                        "Training time limit reached"
                    }
                    .into(),
                );
            }
        }
    }
    world.insert_resource(combat);
}
