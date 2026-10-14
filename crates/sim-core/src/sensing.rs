//! Versioned gameplay sensing. Truth is used only to generate noisy measurements;
//! association and reports consume measurements, never entity identities.
use super::*;
use sim_geo::terrain::{Manifest, Terrain, Visibility};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

type Matrix = [[f64; 4]; 4];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub version: u32,
    pub rationale: String,
    pub epoch_unix: i64,
    pub track_expiry_ticks: u64,
    pub report_interval_ticks: u64,
    pub process_acceleration_mps2: f64,
    pub association_gate: f64,
    pub visual_error_m: f64,
    pub radar_error_m: f64,
    pub bearing_error_deg: f64,
    pub sensor_height_m: f64,
    pub radar_reference_signature: f64,
    pub radar_clutter_factor: f64,
    pub radar_rain_factor: f64,
    pub jammer_power: f64,
    pub emitter_range_m: f64,
}
impl Model {
    pub fn training() -> Self {
        serde_json::from_str(include_str!("../../../data/sensing/training-model.v1.json"))
            .expect("committed sensing model")
    }
    pub fn validate(&self) -> Result<(), String> {
        let positive = [
            self.process_acceleration_mps2,
            self.association_gate,
            self.visual_error_m,
            self.radar_error_m,
            self.bearing_error_deg,
            self.radar_reference_signature,
            self.jammer_power,
            self.emitter_range_m,
        ];
        if self.version != 1
            || self.rationale.is_empty()
            || self.track_expiry_ticks == 0
            || self.report_interval_ticks == 0
            || positive.iter().any(|v| !v.is_finite() || *v <= 0.0)
            || !self.sensor_height_m.is_finite()
            || self.sensor_height_m < 0.0
            || !self.radar_rain_factor.is_finite()
            || self.radar_rain_factor < 0.0
            || !self.radar_clutter_factor.is_finite()
            || !(0.0..=1.0).contains(&self.radar_clutter_factor)
        {
            return Err("invalid sensing model".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Visual,
    Infrared,
    Radar,
    Sigint,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorSuite {
    pub range_m: f64,
    pub identification_range_m: f64,
    pub scan_interval_ticks: u64,
    pub field_of_regard_deg: f64,
    pub modalities: Vec<Modality>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioUpgrade {
    pub bit_rate_bps: u64,
    pub mtu_bytes: usize,
    pub queue_packets: usize,
    pub queue_bytes: usize,
}
impl RadioUpgrade {
    pub fn validate(&self) -> Result<(), String> {
        if self.bit_rate_bps == 0
            || self.mtu_bytes < 512
            || self.queue_packets == 0
            || self.queue_bytes < self.mtu_bytes
        {
            return Err("invalid sensing radio upgrade".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radio_upgrade: Option<RadioUpgrade>,
    pub model: Model,
    pub terrain_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terrain_fixture: Option<sim_geo::terrain::Grid>,
    pub sensors: BTreeMap<Uuid, SensorSuite>,
    pub signatures: BTreeMap<Uuid, Signature>,
    pub weather: Weather,
    pub jammers: Vec<JammingRegion>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub radar: f64,
    pub contrast: f64,
    pub thermal: f64,
    pub emitter_class: String,
}
impl Configuration {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(radio) = &self.radio_upgrade {
            radio.validate()?;
        }
        self.model.validate()?;
        self.weather.validate()?;
        for sensor in self.sensors.values() {
            if !sensor.range_m.is_finite()
                || sensor.range_m <= 0.0
                || !sensor.identification_range_m.is_finite()
                || sensor.identification_range_m < 0.0
                || sensor.identification_range_m > sensor.range_m
                || sensor.scan_interval_ticks == 0
                || !sensor.field_of_regard_deg.is_finite()
                || !(0.0..=360.0).contains(&sensor.field_of_regard_deg)
                || sensor.modalities.is_empty()
            {
                return Err("invalid sensor suite".into());
            }
        }
        for signature in self.signatures.values() {
            if [signature.radar, signature.contrast, signature.thermal]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
                || signature.emitter_class.is_empty()
            {
                return Err("invalid target signature".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Weather {
    pub visibility_m: f64,
    pub cloud_base_m: f64,
    pub cloud_top_m: f64,
    pub cloud_opacity: f64,
    pub precipitation: f64,
    pub thermal_contrast: f64,
}
impl Default for Weather {
    fn default() -> Self {
        Self {
            visibility_m: 100_000.0,
            cloud_base_m: 2000.0,
            cloud_top_m: 4000.0,
            cloud_opacity: 0.0,
            precipitation: 0.0,
            thermal_contrast: 1.0,
        }
    }
}
impl Weather {
    pub fn validate(&self) -> Result<(), String> {
        if !self.visibility_m.is_finite()
            || !(1.0..=1_000_000.0).contains(&self.visibility_m)
            || !self.cloud_base_m.is_finite()
            || !self.cloud_top_m.is_finite()
            || !(0.0..=30_000.0).contains(&self.cloud_base_m)
            || !(self.cloud_base_m..=30_000.0).contains(&self.cloud_top_m)
            || [
                self.cloud_opacity,
                self.precipitation,
                self.thermal_contrast,
            ]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err("invalid weather conditions".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherRegion {
    pub id: String,
    pub center: GeoPose,
    pub radius_m: f64,
    pub priority: i32,
    pub start_tick: u64,
    pub end_tick: Option<u64>,
    pub conditions: Weather,
}
impl WeatherRegion {
    pub fn validate(&self) -> Result<(), String> {
        self.conditions.validate()?;
        if self.id.trim().is_empty()
            || self.id.len() > 100
            || !geo_pose_is_finite(self.center)
            || !self.radius_m.is_finite()
            || !(1.0..=1_000_000.0).contains(&self.radius_m)
            || self.end_tick.is_some_and(|end| end <= self.start_tick)
        {
            return Err("invalid weather region".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub revision: u64,
    pub baseline: Weather,
    pub regions: Vec<WeatherRegion>,
    pub pending: Option<EnvironmentChange>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentChange {
    pub effective_tick: u64,
    pub baseline: Weather,
    pub regions: Vec<WeatherRegion>,
}
impl Environment {
    fn at(&self, position: GeoPose, tick: u64) -> Weather {
        self.regions
            .iter()
            .filter(|r| {
                r.start_tick <= tick
                    && r.end_tick.is_none_or(|end| tick < end)
                    && geodesy::surface_distance_m(position, r.center) <= r.radius_m
            })
            .max_by(|a, b| a.priority.cmp(&b.priority).then_with(|| b.id.cmp(&a.id)))
            .map_or_else(|| self.baseline.clone(), |r| r.conditions.clone())
    }
    pub fn schedule(
        &mut self,
        expected: u64,
        tick: u64,
        baseline: Weather,
        regions: Vec<WeatherRegion>,
    ) -> Result<(), String> {
        if expected != self.revision {
            return Err("weather revision conflict".into());
        }
        baseline.validate()?;
        if regions.len() > 64 {
            return Err("at most 64 weather regions".into());
        }
        let mut ids = BTreeSet::new();
        for r in &regions {
            r.validate()?;
            if !ids.insert(&r.id) {
                return Err("duplicate weather region".into());
            }
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("weather revision exhausted")?;
        self.pending = Some(EnvironmentChange {
            effective_tick: tick,
            baseline,
            regions,
        });
        Ok(())
    }
    fn advance(&mut self, tick: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.effective_tick <= tick)
        {
            let next = self.pending.take().unwrap();
            self.baseline = next.baseline;
            self.regions = next.regions;
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationHypothesis {
    pub domain: Option<Domain>,
    pub side: Option<Side>,
    pub probability: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub hypotheses: Vec<ClassificationHypothesis>,
    pub estimate_tick: u64,
    pub modalities: Vec<Modality>,
    pub covariance: Matrix,
    pub observations: BTreeSet<Uuid>,
    pub velocity: Velocity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bearing {
    pub maximum_range_m: f64,
    pub source: Uuid,
    pub id: Uuid,
    pub receiver_position: GeoPose,
    pub bearing_deg: f64,
    pub uncertainty_deg: f64,
    pub observed_tick: u64,
    pub received_tick: u64,
    pub emitter_class: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherObservation {
    pub source: Uuid,
    pub position: GeoPose,
    pub conditions: Weather,
    pub observed_tick: u64,
    pub received_tick: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalView {
    pub weather_reports: Vec<WeatherObservation>,
    pub bearings: Vec<Bearing>,
    pub radar_emitting: bool,
    pub weather: Weather,
    pub source_modalities: BTreeMap<Uuid, Vec<Modality>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Estimate {
    track: Track,
    origin: GeoPose,
    state: [f64; 4],
    covariance: Matrix,
    predicted_tick: u64,
    identity_tick: u64,
    local_tick: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Resource)]
pub struct Runtime {
    pub configuration: Configuration,
    pub terrain_pin: Option<Manifest>,
    #[serde(skip)]
    pub(crate) terrain: Option<Arc<Terrain>>,
    pub seed: u64,
    pub namespace: Uuid,
    pub environment: Environment,
    pub emissions: BTreeMap<Uuid, bool>,
    tracks: BTreeMap<Uuid, Vec<Estimate>>,
    report_heads: BTreeMap<Uuid, BTreeMap<Uuid, u64>>,
    seen_observations: BTreeMap<Uuid, BTreeMap<Uuid, u64>>,
    bearings: BTreeMap<Uuid, Vec<Bearing>>,
    weather_reports: BTreeMap<Uuid, BTreeMap<Uuid, WeatherObservation>>,
    sequences: BTreeMap<Uuid, u64>,
    pub candidate_count: u64,
    pub terrain_checks: u64,
    #[serde(skip)]
    pub observation_ms: f64,
}
static TERRAIN_CACHE: OnceLock<Mutex<BTreeMap<String, Arc<Terrain>>>> = OnceLock::new();
pub fn terrain_root() -> std::path::PathBuf {
    std::env::var_os("TERRAIN_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "data/generated/terrain/regional".into())
}
fn load_terrain(expected: Option<&Manifest>) -> Result<Arc<Terrain>, String> {
    let root = terrain_root();
    let key = root.display().to_string();
    let cache = TERRAIN_CACHE.get_or_init(Mutex::default);
    if let Some(terrain) = cache.lock().unwrap().get(&key).cloned() {
        if expected.is_some_and(|pin| pin != &terrain.manifest) {
            return Err("installed terrain differs from saved game pin".into());
        }
        return Ok(terrain);
    }
    let terrain = Terrain::load(Path::new(&root), expected)?;
    let mut cache = cache.lock().unwrap();
    if cache.len() >= 2 {
        let first = cache.keys().next().cloned().unwrap();
        cache.remove(&first);
    }
    cache.insert(key, terrain.clone());
    Ok(terrain)
}
impl Runtime {
    pub fn new(configuration: Configuration, seed: u64, namespace: Uuid) -> Result<Self, String> {
        configuration.validate()?;
        let terrain = if let Some(grid) = &configuration.terrain_fixture {
            Some(grid.terrain()?)
        } else if configuration.terrain_required {
            Some(load_terrain(None)?)
        } else {
            None
        };
        Ok(Self {
            environment: Environment {
                revision: 0,
                baseline: configuration.weather.clone(),
                regions: vec![],
                pending: None,
            },
            emissions: configuration
                .sensors
                .iter()
                .filter(|(_, s)| s.modalities.contains(&Modality::Radar))
                .map(|(id, _)| (*id, true))
                .collect(),
            terrain_pin: terrain.as_ref().map(|t| t.manifest.clone()),
            terrain,
            configuration,
            seed,
            namespace,
            tracks: BTreeMap::new(),
            report_heads: BTreeMap::new(),
            seen_observations: BTreeMap::new(),
            bearings: BTreeMap::new(),
            weather_reports: BTreeMap::new(),
            sequences: BTreeMap::new(),
            candidate_count: 0,
            terrain_checks: 0,
            observation_ms: 0.0,
        })
    }
    pub fn restore_terrain(&mut self) -> Result<(), String> {
        self.configuration.validate()?;
        self.terrain = if let Some(grid) = &self.configuration.terrain_fixture {
            let terrain = grid.terrain()?;
            if self.terrain_pin.as_ref() != Some(&terrain.manifest) {
                return Err("synthetic terrain pin mismatch".into());
            }
            Some(terrain)
        } else if self.configuration.terrain_required {
            Some(load_terrain(Some(
                self.terrain_pin.as_ref().ok_or("missing terrain pin")?,
            ))?)
        } else {
            None
        };
        Ok(())
    }
    fn next_id(&mut self, owner: Uuid, kind: &str) -> Uuid {
        let sequence = self.sequences.entry(owner).or_default();
        *sequence += 1;
        let namespace = Uuid::new_v5(&self.namespace, owner.as_bytes());
        Uuid::new_v5(&namespace, format!("{kind}:{sequence}").as_bytes())
    }
    pub fn set_emission(&mut self, id: Uuid, emitting: bool) -> Result<(), String> {
        let state = self
            .emissions
            .get_mut(&id)
            .ok_or("platform has no controllable radar")?;
        *state = emitting;
        Ok(())
    }
    pub fn tracks_for(&self, owner: Uuid, tick: u64) -> Vec<Track> {
        self.tracks
            .get(&owner)
            .into_iter()
            .flatten()
            .filter(|t| {
                tick.saturating_sub(t.track.observed_tick)
                    <= self.configuration.model.track_expiry_ticks
            })
            .map(|estimate| {
                let mut estimate = estimate.clone();
                predict(
                    &mut estimate,
                    tick,
                    self.configuration.model.process_acceleration_mps2,
                );
                let mut track = estimate.track;
                track.position = pose(estimate.origin, estimate.state[0], estimate.state[1]);
                track.uncertainty_m = 2.448
                    * estimate.covariance[0][0]
                        .max(estimate.covariance[1][1])
                        .sqrt();
                track.identity_confidence *= (1.0
                    - tick.saturating_sub(estimate.identity_tick) as f32
                        / self.configuration.model.track_expiry_ticks as f32)
                    .max(0.0);
                if track.identity_confidence < 0.8 {
                    track.target_side = None;
                }
                if let Some(evidence) = &mut track.evidence {
                    for hypothesis in &mut evidence.hypotheses {
                        hypothesis.probability *= (1.0
                            - tick.saturating_sub(estimate.identity_tick) as f32
                                / self.configuration.model.track_expiry_ticks as f32)
                            .max(0.0);
                    }
                    // A small rolling evidence window keeps low-bandwidth reports bounded.
                    // Reports always use covariance intersection; omitted older IDs cannot
                    // turn a forwarded estimate into independent evidence.
                    let seen = self.seen_observations.get(&owner);
                    let mut ids: Vec<_> = evidence.observations.iter().copied().collect();
                    ids.sort_by(|a, b| {
                        seen.and_then(|s| s.get(b))
                            .cmp(&seen.and_then(|s| s.get(a)))
                            .then(a.cmp(b))
                    });
                    evidence.observations = ids.into_iter().take(8).collect();
                    evidence.estimate_tick = tick;
                    evidence.covariance = estimate.covariance;
                    evidence.velocity = Velocity {
                        north_mps: estimate.state[2],
                        east_mps: estimate.state[3],
                        climb_mps: 0.0,
                    };
                }
                track
            })
            .collect()
    }
    pub fn current_bearings(&self, owner: Uuid, tick: u64) -> Vec<Bearing> {
        self.bearings
            .get(&owner)
            .into_iter()
            .flatten()
            .filter(|bearing| bearing.observed_tick == tick)
            .cloned()
            .collect()
    }
    pub fn current_tracks(&self, owner: Uuid, tick: u64) -> Vec<Track> {
        let local: BTreeSet<_> = self
            .tracks
            .get(&owner)
            .into_iter()
            .flatten()
            .filter(|e| e.local_tick == Some(tick))
            .map(|e| e.track.track_id)
            .collect();
        self.tracks_for(owner, tick)
            .into_iter()
            .filter(|t| local.contains(&t.track_id))
            .collect()
    }
    pub fn local_view(&self, owner: Uuid, position: GeoPose, tick: u64) -> LocalView {
        LocalView {
            weather_reports: self
                .weather_reports
                .get(&owner)
                .into_iter()
                .flat_map(|reports| reports.values())
                .filter(|report| {
                    tick.saturating_sub(report.observed_tick)
                        <= self.configuration.model.track_expiry_ticks
                })
                .cloned()
                .collect(),
            bearings: self
                .bearings
                .get(&owner)
                .into_iter()
                .flatten()
                .filter(|b| tick.saturating_sub(b.observed_tick) <= 30)
                .cloned()
                .collect(),
            radar_emitting: self.emissions.get(&owner).copied().unwrap_or(false),
            weather: self.environment.at(position, tick),
            source_modalities: self
                .tracks_for(owner, tick)
                .iter()
                .filter_map(|t| {
                    t.evidence
                        .as_ref()
                        .map(|e| (t.track_id, e.modalities.clone()))
                })
                .collect(),
        }
    }
    pub fn receive(&mut self, owner: Uuid, mut track: Track, tick: u64, local: bool) -> bool {
        if track.observed_tick > tick
            || tick.saturating_sub(track.observed_tick)
                > self.configuration.model.track_expiry_ticks
            || !geo_pose_is_finite(track.position)
            || !track.identity_confidence.is_finite()
            || !(0.0..=1.0).contains(&track.identity_confidence)
            || !track.uncertainty_m.is_finite()
            || track.uncertainty_m <= 0.0
        {
            return false;
        }
        let original_id = track.track_id;
        if !local
            && self
                .report_heads
                .get(&owner)
                .and_then(|heads| heads.get(&original_id))
                .is_some_and(|head| *head >= track.observed_tick)
        {
            return false;
        }

        let evidence = track.evidence.clone().unwrap_or_else(|| Evidence {
            hypotheses: vec![ClassificationHypothesis {
                domain: track.observed_domain,
                side: track.target_side,
                probability: track.identity_confidence,
            }],
            estimate_tick: track.observed_tick,
            modalities: vec![Modality::Radar],
            covariance: diagonal((track.uncertainty_m / 2.448).powi(2)),
            observations: BTreeSet::from([original_id]),
            velocity: Velocity::default(),
        });
        if evidence.estimate_tick > tick
            || evidence.estimate_tick < track.observed_tick
            || !valid_covariance(evidence.covariance)
            || !evidence.velocity.north_mps.is_finite()
            || !evidence.velocity.east_mps.is_finite()
            || evidence
                .hypotheses
                .iter()
                .any(|h| !h.probability.is_finite() || !(0.0..=1.0).contains(&h.probability))
            || evidence.hypotheses.len() > 16
            || evidence.observations.is_empty()
            || evidence.observations.len() > 256
        {
            return false;
        }
        if evidence.observations.iter().all(|id| {
            self.seen_observations
                .get(&owner)
                .is_some_and(|seen| seen.contains_key(id))
        }) {
            return false;
        }
        let list = self.tracks.entry(owner).or_default();
        if list.iter().any(|e| {
            e.track
                .evidence
                .as_ref()
                .is_some_and(|old| evidence.observations.is_subset(&old.observations))
        }) {
            return false;
        }
        // Associate at observation time. Out-of-order data never rewinds a newer estimate.
        let mut candidates: Vec<_> = list
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.predicted_tick <= evidence.estimate_tick
                    && !(e.track.target_side.is_some()
                        && track.target_side.is_some()
                        && e.track.target_side != track.target_side)
                    && !(e.track.observed_domain.is_some()
                        && track.observed_domain.is_some()
                        && e.track.observed_domain != track.observed_domain)
            })
            .filter_map(|(i, e)| {
                let mut predicted = e.clone();
                predict(
                    &mut predicted,
                    evidence.estimate_tick,
                    self.configuration.model.process_acceleration_mps2,
                );
                let measured = offset(predicted.origin, track.position);
                let dn = measured[0] - predicted.state[0];
                let de = measured[1] - predicted.state[1];
                let s = [
                    [
                        predicted.covariance[0][0] + evidence.covariance[0][0],
                        predicted.covariance[0][1] + evidence.covariance[0][1],
                    ],
                    [
                        predicted.covariance[1][0] + evidence.covariance[1][0],
                        predicted.covariance[1][1] + evidence.covariance[1][1],
                    ],
                ];
                let score = quadratic(s, [dn, de])?;
                // Rank gated candidates by normalized Gaussian likelihood.
                // Mahalanobis distance alone favors very broad estimates and
                // makes a precise return ambiguous with unrelated coarse fixes.
                let determinant = s[0][0] * s[1][1] - s[0][1] * s[1][0];
                (score <= self.configuration.model.association_gate && determinant > 0.0)
                    .then_some((i, score + determinant.ln(), e.track.track_id))
            })
            .collect();
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)));
        let matched = if candidates.len() > 1 && candidates[1].1 - candidates[0].1 < 1.0 {
            None
        } else {
            candidates.first().map(|c| c.0)
        };
        self.report_heads
            .entry(owner)
            .or_default()
            .insert(original_id, track.observed_tick);
        for id in &evidence.observations {
            self.seen_observations
                .entry(owner)
                .or_default()
                .entry(*id)
                .or_insert(track.observed_tick);
        }
        track.received_tick = tick;
        track.evidence = Some(evidence.clone());
        if let Some(index) = matched {
            let estimate = &mut self.tracks.get_mut(&owner).unwrap()[index];
            predict(
                estimate,
                evidence.estimate_tick,
                self.configuration.model.process_acceleration_mps2,
            );
            let measured = offset(estimate.origin, track.position);
            // Conservative covariance intersection for reports: forwarded tracks are not
            // independent raw observations even when their retained ID sets differ.
            if local {
                update(estimate, measured, evidence.covariance);
            } else {
                intersection(estimate, measured, &evidence);
            }
            let old_evidence = estimate.track.evidence.as_mut().unwrap();
            old_evidence.observations.extend(evidence.observations);
            while old_evidence.observations.len() > 256 {
                let seen = self.seen_observations.get(&owner);
                let first = *old_evidence
                    .observations
                    .iter()
                    .min_by_key(|id| seen.and_then(|s| s.get(id)))
                    .unwrap();
                old_evidence.observations.remove(&first);
            }
            for mode in evidence.modalities {
                if !old_evidence.modalities.contains(&mode) {
                    old_evidence.modalities.push(mode);
                }
            }
            for hypothesis in evidence.hypotheses {
                if let Some(existing) = old_evidence
                    .hypotheses
                    .iter_mut()
                    .find(|h| h.domain == hypothesis.domain && h.side == hypothesis.side)
                {
                    existing.probability = existing.probability.max(hypothesis.probability);
                } else if old_evidence.hypotheses.len() < 16 {
                    old_evidence.hypotheses.push(hypothesis);
                }
            }
            if old_evidence
                .hypotheses
                .iter()
                .any(|h| h.domain.is_some() || h.side.is_some())
            {
                old_evidence
                    .hypotheses
                    .retain(|h| h.domain.is_some() || h.side.is_some());
            }
            old_evidence.covariance = estimate.covariance;
            estimate.track.observed_tick = track.observed_tick;
            estimate.track.received_tick = tick;
            estimate.track.identity_confidence = estimate
                .track
                .identity_confidence
                .max(track.identity_confidence);
            if track.identity_confidence >= 0.8 {
                estimate.identity_tick = track.observed_tick;
                estimate.track.target_side = track.target_side;
                estimate.track.observed_domain = track.observed_domain;
            }
            estimate.origin.altitude_m = track.position.altitude_m;
            estimate.track.position = pose(estimate.origin, estimate.state[0], estimate.state[1]);
            if local {
                estimate.local_tick = Some(tick);
            }
        } else {
            let id = self.next_id(owner, "track");
            track.track_id = id;
            let estimate = Estimate {
                origin: track.position,
                state: [
                    0.0,
                    0.0,
                    evidence.velocity.north_mps,
                    evidence.velocity.east_mps,
                ],
                covariance: evidence.covariance,
                predicted_tick: evidence.estimate_tick,
                identity_tick: track.observed_tick,
                local_tick: local.then_some(tick),
                track,
            };
            self.tracks.entry(owner).or_default().push(estimate);
        }
        true
    }
    pub fn receive_bearing(&mut self, owner: Uuid, mut bearing: Bearing, tick: u64) -> bool {
        if bearing.observed_tick > tick
            || tick.saturating_sub(bearing.observed_tick) > 30
            || !geo_pose_is_finite(bearing.receiver_position)
            || !bearing.maximum_range_m.is_finite()
            || !(1.0..=1_000_000.0).contains(&bearing.maximum_range_m)
            || !bearing.bearing_deg.is_finite()
            || !bearing.uncertainty_deg.is_finite()
            || !(0.01..=90.0).contains(&bearing.uncertainty_deg)
            || bearing.emitter_class.is_empty()
        {
            return false;
        }
        let list = self.bearings.entry(owner).or_default();
        if list.iter().any(|b| b.id == bearing.id) {
            return false;
        }
        bearing.received_tick = tick;
        list.push(bearing.clone());
        let others: Vec<_> = list
            .iter()
            .filter(|b| {
                b.source != bearing.source
                    && b.emitter_class == bearing.emitter_class
                    && b.observed_tick.abs_diff(bearing.observed_tick) <= 5
                    && geodesy::surface_distance_m(b.receiver_position, bearing.receiver_position)
                        > 500.0
            })
            .cloned()
            .collect();
        // Several simultaneous emitters with the same coarse class cannot be
        // paired by choosing an arbitrary bearing. Preserve bearing regions
        // until every contributing receiver has one compatible observation.
        let mut newest: BTreeMap<Uuid, u64> = BTreeMap::new();
        for observation in list
            .iter()
            .filter(|observation| observation.emitter_class == bearing.emitter_class)
        {
            newest
                .entry(observation.source)
                .and_modify(|tick| *tick = (*tick).max(observation.observed_tick))
                .or_insert(observation.observed_tick);
        }
        if newest.iter().any(|(source, newest_tick)| {
            newest_tick.abs_diff(bearing.observed_tick) <= 5
                && list
                    .iter()
                    .filter(|observation| {
                        observation.source == *source
                            && observation.emitter_class == bearing.emitter_class
                            && observation.observed_tick == *newest_tick
                    })
                    .count()
                    > 1
        }) {
            return true;
        }
        let mut latest = BTreeMap::new();
        for other in others {
            let replace = latest
                .get(&other.source)
                .is_none_or(|old: &Bearing| old.observed_tick < other.observed_tick);
            if replace {
                latest.insert(other.source, other);
            }
        }
        let others: Vec<_> = latest.into_values().collect();
        let mut contributors = others;
        contributors.push(bearing.clone());
        if let Some((position, covariance)) = fit_bearings(&contributors) {
            let observed_tick = contributors.iter().map(|b| b.observed_tick).max().unwrap();
            let ids: BTreeSet<_> = contributors.iter().map(|b| b.id).collect();
            let id = Uuid::new_v5(
                &bearing.id,
                ids.iter()
                    .flat_map(|id| id.as_bytes().iter().copied())
                    .collect::<Vec<_>>()
                    .as_slice(),
            );
            self.receive(
                owner,
                Track {
                    track_id: id,
                    target_side: None,
                    position,
                    identity_confidence: 0.45,
                    observed_tick,
                    received_tick: tick,
                    observed_sidc: "100101000000000000000000000000".into(),
                    uncertainty_m: 2.448 * covariance[0][0].max(covariance[1][1]).sqrt(),
                    assessed_destroyed: None,
                    observed_domain: None,
                    evidence: Some(Evidence {
                        hypotheses: vec![ClassificationHypothesis {
                            domain: None,
                            side: None,
                            probability: 0.45,
                        }],
                        estimate_tick: observed_tick,
                        modalities: vec![Modality::Sigint],
                        covariance,
                        observations: ids,
                        velocity: Velocity::default(),
                    }),
                },
                tick,
                false,
            );
        }
        true
    }
    pub fn observe(&mut self, tick: u64, platforms: &[Platform], jammers: &[JammingRegion]) {
        let started = std::time::Instant::now();
        self.environment.advance(tick);
        self.candidate_count = 0;
        self.terrain_checks = 0;
        let expiry = self.configuration.model.track_expiry_ticks;
        for heads in self.report_heads.values_mut() {
            heads.retain(|_, observed| tick.saturating_sub(*observed) <= expiry);
        }
        for seen in self.seen_observations.values_mut() {
            seen.retain(|_, observed| tick.saturating_sub(*observed) <= expiry);
        }
        for list in self.tracks.values_mut() {
            list.retain(|t| tick.saturating_sub(t.track.observed_tick) <= expiry);
        }
        for reports in self.weather_reports.values_mut() {
            reports.retain(|_, report| tick.saturating_sub(report.observed_tick) <= expiry);
        }
        for list in self.bearings.values_mut() {
            list.retain(|b| tick.saturating_sub(b.observed_tick) <= 30);
        }
        // Stable latitude index narrows candidates before range/terrain checks.
        let mut spatial: Vec<_> = platforms.iter().filter(|p| !p.destroyed).collect();
        spatial.sort_by(|a, b| {
            a.pose
                .latitude_deg
                .total_cmp(&b.pose.latitude_deg)
                .then(a.id.cmp(&b.id))
        });
        for observer in platforms.iter().filter(|p| !p.destroyed) {
            let Some(sensor) = self.configuration.sensors.get(&observer.id).cloned() else {
                continue;
            };
            if tick != 1 && !tick.is_multiple_of(sensor.scan_interval_ticks) {
                continue;
            }
            let margin = sensor.range_m / 110_000.0;
            let start = spatial
                .partition_point(|p| p.pose.latitude_deg < observer.pose.latitude_deg - margin);
            let stop = spatial
                .partition_point(|p| p.pose.latitude_deg <= observer.pose.latitude_deg + margin);
            for target in &spatial[start..stop] {
                if observer.id == target.id {
                    continue;
                }
                self.candidate_count += 1;
                let distance = geodesy::slant_distance_m(observer.pose, target.pose);
                if distance > sensor.range_m {
                    continue;
                }
                let mut source = observer.pose;
                source.altitude_m += self.configuration.model.sensor_height_m;
                if !geodesy::has_geometric_line_of_sight(source, target.pose) {
                    continue;
                }
                if let Some(terrain) = &self.terrain {
                    self.terrain_checks += 1;
                    if terrain.visibility(
                        [source.latitude_deg, source.longitude_deg, source.altitude_m],
                        [
                            target.pose.latitude_deg,
                            target.pose.longitude_deg,
                            target.pose.altitude_m,
                        ],
                    ) != Visibility::Clear
                    {
                        continue;
                    }
                }
                let bearing = bearing_deg(observer.pose, target.pose);
                let heading = observer
                    .velocity
                    .east_mps
                    .atan2(observer.velocity.north_mps)
                    .to_degrees();
                if angle_difference(bearing, heading).abs() > sensor.field_of_regard_deg / 2.0 {
                    continue;
                }
                let Some(signature) = self.configuration.signatures.get(&target.id).cloned() else {
                    continue;
                };
                for mode in &sensor.modalities {
                    let key = format!(
                        "{tick}:{}:{mode:?}:{:.9}:{:.9}:{:.3}",
                        observer.id,
                        target.pose.latitude_deg,
                        target.pose.longitude_deg,
                        target.pose.altitude_m
                    );
                    if *mode == Modality::Sigint {
                        if observer.side == target.side {
                            continue;
                        }
                        if !self.emissions.get(&target.id).copied().unwrap_or(false)
                            || distance > self.configuration.model.emitter_range_m
                        {
                            continue;
                        }
                        let error = self.configuration.model.bearing_error_deg;
                        let id = self.next_id(observer.id, "bearing");
                        let measured = Bearing {
                            maximum_range_m: sensor
                                .range_m
                                .min(self.configuration.model.emitter_range_m),
                            source: Uuid::new_v5(&self.namespace, observer.id.as_bytes()),
                            id,
                            receiver_position: observer.pose,
                            bearing_deg: (bearing + normal(self.seed, &key, 0) * error)
                                .rem_euclid(360.0),
                            uncertainty_deg: error,
                            observed_tick: tick,
                            received_tick: tick,
                            emitter_class: signature.emitter_class.clone(),
                        };
                        self.receive_bearing(observer.id, measured, tick);
                        continue;
                    }
                    if observer.side == target.side {
                        continue;
                    }
                    if *mode == Modality::Radar
                        && !self.emissions.get(&observer.id).copied().unwrap_or(false)
                    {
                        continue;
                    }
                    let weather = self.environment.at(observer.pose, tick);
                    let target_weather = self.environment.at(target.pose, tick);
                    let weather = Weather {
                        visibility_m: weather.visibility_m.min(target_weather.visibility_m),
                        cloud_opacity: weather.cloud_opacity.max(target_weather.cloud_opacity),
                        precipitation: weather.precipitation.max(target_weather.precipitation),
                        thermal_contrast: weather
                            .thermal_contrast
                            .min(target_weather.thermal_contrast),
                        ..weather
                    };
                    let probability = detection_probability(
                        *mode,
                        distance,
                        &sensor,
                        &signature,
                        &weather,
                        source,
                        target.pose,
                        target.velocity,
                        &self.configuration.model,
                        jammers,
                        tick,
                    );
                    if uniform(self.seed, &key, 1) > probability {
                        continue;
                    }
                    let error = if *mode == Modality::Radar {
                        self.configuration.model.radar_error_m
                    } else {
                        self.configuration.model.visual_error_m
                    };
                    let mut position = geodesy::advance_position(
                        target.pose,
                        normal(self.seed, &key, 2) * error,
                        normal(self.seed, &key, 3) * error,
                        normal(self.seed, &key, 4) * error,
                    )
                    .unwrap();
                    position.altitude_m = position.altitude_m.max(0.0);
                    let classified =
                        distance <= sensor.identification_range_m && probability >= 0.7;
                    let id = self.next_id(observer.id, "observation");
                    self.receive(
                        observer.id,
                        Track {
                            track_id: id,
                            target_side: classified.then_some(target.side),
                            position,
                            identity_confidence: if classified { 0.9 } else { 0.45 },
                            observed_tick: tick,
                            received_tick: tick,
                            observed_sidc: "100101000000000000000000000000".into(),
                            uncertainty_m: 2.448 * error,
                            assessed_destroyed: None,
                            observed_domain: classified.then_some(target.domain),
                            evidence: Some(Evidence {
                                hypotheses: vec![ClassificationHypothesis {
                                    domain: classified.then_some(target.domain),
                                    side: classified.then_some(target.side),
                                    probability: if classified { 0.9 } else { 0.45 },
                                }],
                                estimate_tick: tick,
                                modalities: vec![*mode],
                                covariance: diagonal(error * error),
                                observations: BTreeSet::from([id]),
                                velocity: Velocity::default(),
                            }),
                        },
                        tick,
                        true,
                    );
                }
            }
        }
        self.observation_ms = started.elapsed().as_secs_f64() * 1000.0;
    }
}
#[derive(Clone)]
pub struct Platform {
    pub id: Uuid,
    pub side: Side,
    pub domain: Domain,
    pub pose: GeoPose,
    pub velocity: Velocity,
    pub destroyed: bool,
}
fn uniform(seed: u64, key: &str, channel: u64) -> f64 {
    let mut h = seed ^ channel.wrapping_mul(0x9e3779b97f4a7c15);
    for b in key.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58476d1ce4e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d049bb133111eb);
    h ^= h >> 31;
    ((h >> 11) as f64 + 0.5) / ((1u64 << 53) as f64)
}
fn normal(seed: u64, key: &str, channel: u64) -> f64 {
    (-2.0 * uniform(seed, key, channel * 2 + 11).ln()).sqrt()
        * (std::f64::consts::TAU * uniform(seed, key, channel * 2 + 12)).cos()
}
fn angle_difference(a: f64, b: f64) -> f64 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}
fn bearing_deg(a: GeoPose, b: GeoPose) -> f64 {
    let delta = (b.longitude_deg - a.longitude_deg).to_radians();
    let a = a.latitude_deg.to_radians();
    let b = b.latitude_deg.to_radians();
    (delta.sin() * b.cos())
        .atan2(a.cos() * b.sin() - a.sin() * b.cos() * delta.cos())
        .to_degrees()
        .rem_euclid(360.0)
}
/// Low-order solar geometry: sufficient for deterministic fictional day/night sensing.
pub fn solar_elevation(epoch: i64, tick: u64, p: GeoPose) -> f64 {
    let days = (epoch as f64 + tick as f64) / 86400.0;
    let year_day = (days - 10_957.0).rem_euclid(365.2425) + 1.0;
    let declination = (23.44_f64.to_radians()
        * ((std::f64::consts::TAU * (year_day - 80.0) / 365.2425).sin()))
    .sin()
    .asin();
    let hour = (days.fract() * 360.0 + p.longitude_deg - 180.0).to_radians();
    let lat = p.latitude_deg.to_radians();
    (lat.sin() * declination.sin() + lat.cos() * declination.cos() * hour.cos())
        .asin()
        .to_degrees()
}
#[allow(clippy::too_many_arguments)]
fn detection_probability(
    mode: Modality,
    distance: f64,
    sensor: &SensorSuite,
    signature: &Signature,
    weather: &Weather,
    source: GeoPose,
    target: GeoPose,
    velocity: Velocity,
    model: &Model,
    jammers: &[JammingRegion],
    tick: u64,
) -> f64 {
    let cloud = if source.altitude_m.min(target.altitude_m) <= weather.cloud_top_m
        && source.altitude_m.max(target.altitude_m) >= weather.cloud_base_m
    {
        1.0 - weather.cloud_opacity
    } else {
        1.0
    };
    let score = match mode {
        Modality::Visual => {
            signature.contrast
                * (-distance / weather.visibility_m).exp()
                * cloud
                * ((solar_elevation(model.epoch_unix, tick, source) + 6.0) / 20.0).clamp(0.02, 1.0)
                * sensor.range_m
                / distance.max(1.0)
        }
        Modality::Infrared => {
            signature.thermal
                * weather.thermal_contrast
                * (-distance / (weather.visibility_m * 2.0)).exp()
                * cloud
                * sensor.range_m
                / distance.max(1.0)
        }
        Modality::Radar => {
            let aspect = angle_difference(
                bearing_deg(target, source),
                velocity.east_mps.atan2(velocity.north_mps).to_degrees(),
            )
            .to_radians();
            let clutter = if source.altitude_m > target.altitude_m {
                model.radar_clutter_factor
            } else {
                1.0
            };
            let interference: f64 = jammers
                .iter()
                .filter(|j| {
                    j.active_from_tick <= tick
                        && j.active_until_tick.is_none_or(|end| tick < end)
                        && j.band.lower_hz <= 1_215_000_000
                        && j.band.upper_hz >= 960_000_000
                })
                .map(|j| {
                    model.jammer_power
                        * j.jammed
                        * (j.radius_m / geodesy::slant_distance_m(source, j.center).max(j.radius_m))
                            .powi(2)
                })
                .sum();
            signature.radar / model.radar_reference_signature
                * (0.5 + 0.5 * aspect.sin().abs())
                * (sensor.range_m / distance.max(1.0)).powi(4)
                * clutter
                * (-weather.precipitation * model.radar_rain_factor * distance / 10000.0).exp()
                / (1.0 + interference)
        }
        Modality::Sigint => 0.0,
    };
    score.clamp(0.0, 1.0)
}
fn offset(origin: GeoPose, p: GeoPose) -> [f64; 2] {
    let d = geodesy::surface_distance_m(origin, p);
    let b = bearing_deg(origin, p).to_radians();
    [d * b.cos(), d * b.sin()]
}
fn pose(origin: GeoPose, n: f64, e: f64) -> GeoPose {
    geodesy::advance_position(origin, n, e, 0.0).unwrap_or(origin)
}
fn diagonal(variance: f64) -> Matrix {
    [
        [variance, 0.0, 0.0, 0.0],
        [0.0, variance, 0.0, 0.0],
        [0.0, 0.0, 2500.0, 0.0],
        [0.0, 0.0, 0.0, 2500.0],
    ]
}
fn valid_covariance(m: Matrix) -> bool {
    if !m.iter().flatten().all(|v| v.is_finite())
        || !(0..4).all(|i| (0..4).all(|j| (m[i][j] - m[j][i]).abs() < 1e-6))
    {
        return false;
    }
    let mut lower = [[0.0_f64; 4]; 4];
    for i in 0..4 {
        for j in 0..=i {
            let residual = m[i][j] - (0..j).map(|k| lower[i][k] * lower[j][k]).sum::<f64>();
            if i == j {
                if residual <= 0.0 {
                    return false;
                }
                lower[i][j] = residual.sqrt();
            } else {
                lower[i][j] = residual / lower[j][j];
            }
        }
    }
    true
}
fn inverse(mut m: Matrix) -> Option<Matrix> {
    let mut out = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }));
    for i in 0..4 {
        let pivot = (i..4).max_by(|a, b| m[*a][i].abs().total_cmp(&m[*b][i].abs()))?;
        m.swap(i, pivot);
        out.swap(i, pivot);
        let d = m[i][i];
        if d.abs() < 1e-12 {
            return None;
        }
        for j in 0..4 {
            m[i][j] /= d;
            out[i][j] /= d;
        }
        for k in 0..4 {
            if k == i {
                continue;
            }
            let f = m[k][i];
            for j in 0..4 {
                m[k][j] -= f * m[i][j];
                out[k][j] -= f * out[i][j];
            }
        }
    }
    Some(out)
}
fn quadratic(m: [[f64; 2]; 2], v: [f64; 2]) -> Option<f64> {
    let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
    (det > 0.0).then(|| {
        (m[1][1] * v[0] * v[0] - 2.0 * m[0][1] * v[0] * v[1] + m[0][0] * v[1] * v[1]) / det
    })
}
fn predict(e: &mut Estimate, tick: u64, acceleration: f64) {
    let dt = tick.saturating_sub(e.predicted_tick) as f64;
    if dt == 0.0 {
        return;
    }
    e.state[0] += dt * e.state[2];
    e.state[1] += dt * e.state[3];
    let f: [[f64; 4]; 4] = [
        [1.0, 0.0, dt, 0.0],
        [0.0, 1.0, 0.0, dt],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let p = e.covariance;
    e.covariance = std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            (0..4)
                .map(|a| (0..4).map(|b| f[i][a] * p[a][b] * f[j][b]).sum::<f64>())
                .sum()
        })
    });
    let q = acceleration.powi(2);
    for (pos, vel) in [(0, 2), (1, 3)] {
        e.covariance[pos][pos] += q * dt.powi(4) / 4.0;
        e.covariance[pos][vel] += q * dt.powi(3) / 2.0;
        e.covariance[vel][pos] += q * dt.powi(3) / 2.0;
        e.covariance[vel][vel] += q * dt.powi(2);
    }
    e.predicted_tick = tick;
}
fn update(e: &mut Estimate, z: [f64; 2], r: Matrix) {
    let p = e.covariance;
    let s = [
        [p[0][0] + r[0][0], p[0][1] + r[0][1]],
        [p[1][0] + r[1][0], p[1][1] + r[1][1]],
    ];
    let det = s[0][0] * s[1][1] - s[0][1] * s[1][0];
    if det <= 0.0 {
        return;
    }
    let inv = [
        [s[1][1] / det, -s[0][1] / det],
        [-s[1][0] / det, s[0][0] / det],
    ];
    let k: [[f64; 2]; 4] =
        std::array::from_fn(|i| std::array::from_fn(|j| p[i][0] * inv[0][j] + p[i][1] * inv[1][j]));
    let residual = [z[0] - e.state[0], z[1] - e.state[1]];
    for (i, x) in e.state.iter_mut().enumerate() {
        *x += k[i][0] * residual[0] + k[i][1] * residual[1];
    }
    e.covariance = std::array::from_fn(|i| {
        std::array::from_fn(|j| p[i][j] - k[i][0] * p[0][j] - k[i][1] * p[1][j])
    });
}
fn intersection(e: &mut Estimate, z: [f64; 2], evidence: &Evidence) {
    let (Some(a), Some(b)) = (inverse(e.covariance), inverse(evidence.covariance)) else {
        return;
    };
    let other = [
        z[0],
        z[1],
        evidence.velocity.north_mps,
        evidence.velocity.east_mps,
    ];
    let mut best = None;
    for step in 0..=20 {
        let w = step as f64 / 20.0;
        let info =
            std::array::from_fn(|i| std::array::from_fn(|j| w * a[i][j] + (1.0 - w) * b[i][j]));
        let Some(p) = inverse(info) else {
            continue;
        };
        let cost = p[0][0] * p[1][1] - p[0][1] * p[1][0];
        if best.as_ref().is_none_or(|(old, _, _)| cost < *old) {
            best = Some((cost, w, p));
        }
    }
    if let Some((_, w, p)) = best {
        let rhs: [f64; 4] = std::array::from_fn(|i| {
            (0..4)
                .map(|j| w * a[i][j] * e.state[j] + (1.0 - w) * b[i][j] * other[j])
                .sum()
        });
        e.state = std::array::from_fn(|i| (0..4).map(|j| p[i][j] * rhs[j]).sum());
        e.covariance = p;
    }
}
fn triangulate(a: &Bearing, b: &Bearing) -> Option<(GeoPose, f64)> {
    let origin = a.receiver_position;
    let delta = offset(origin, b.receiver_position);
    let aa = a.bearing_deg.to_radians();
    let bb = b.bearing_deg.to_radians();
    let u = [aa.cos(), aa.sin()];
    let v = [bb.cos(), bb.sin()];
    let cross = u[0] * v[1] - u[1] * v[0];
    if cross.abs() < 0.2 {
        return None;
    }
    let t = (delta[0] * v[1] - delta[1] * v[0]) / cross;
    let s = (delta[0] * u[1] - delta[1] * u[0]) / cross;
    if !(0.0..=200_000.0).contains(&t) || !(0.0..=200_000.0).contains(&s) {
        return None;
    }
    let variance = ((t * a.uncertainty_deg.to_radians()).powi(2)
        + (s * b.uncertainty_deg.to_radians()).powi(2))
        / cross.powi(2);
    Some((pose(origin, t * u[0], t * u[1]), variance.max(100.0)))
}

fn fit_bearings(bearings: &[Bearing]) -> Option<(GeoPose, Matrix)> {
    if bearings.len() < 2 {
        return None;
    }
    let origin = bearings[0].receiver_position;
    let initial = triangulate(&bearings[0], &bearings[1])?.0;
    let mut position = offset(origin, initial);
    let mut covariance = diagonal(100.0);
    for _ in 0..3 {
        let mut normal = [[0.0; 2]; 2];
        let mut rhs = [0.0; 2];
        for b in bearings {
            let receiver = offset(origin, b.receiver_position);
            let theta = b.bearing_deg.to_radians();
            let n = [-theta.sin(), theta.cos()];
            let range = (position[0] - receiver[0]).hypot(position[1] - receiver[1]);
            let weight = 1.0 / (range * b.uncertainty_deg.to_radians()).powi(2).max(100.0);
            let line = n[0] * receiver[0] + n[1] * receiver[1];
            for i in 0..2 {
                rhs[i] += weight * n[i] * line;
                for j in 0..2 {
                    normal[i][j] += weight * n[i] * n[j];
                }
            }
        }
        let det = normal[0][0] * normal[1][1] - normal[0][1] * normal[1][0];
        if det / (normal[0][0] + normal[1][1]).powi(2) < 0.01 {
            return None;
        }
        covariance[0][0] = normal[1][1] / det;
        covariance[1][1] = normal[0][0] / det;
        covariance[0][1] = -normal[0][1] / det;
        covariance[1][0] = covariance[0][1];
        position = [
            covariance[0][0] * rhs[0] + covariance[0][1] * rhs[1],
            covariance[1][0] * rhs[0] + covariance[1][1] * rhs[1],
        ];
    }
    let fix = pose(origin, position[0], position[1]);
    for b in bearings {
        if geodesy::surface_distance_m(fix, b.receiver_position) > b.maximum_range_m
            || angle_difference(bearing_deg(b.receiver_position, fix), b.bearing_deg).abs()
                > b.uncertainty_deg * 3.0
        {
            return None;
        }
    }
    Some((fix, covariance))
}

#[cfg(test)]
mod tests;

impl Simulation {
    pub fn configure_sensing(
        &mut self,
        configuration: Configuration,
        seed: u64,
        namespace: Uuid,
    ) -> Result<(), String> {
        let mut runtime = Runtime::new(configuration, seed, namespace)?;
        if let Some(radio) = &runtime.configuration.radio_upgrade {
            self.upgrade_training_radio(radio)?;
        }
        let tick = self.tick();
        // Preserve already known observations during an explicit rules migration.
        for (owner, tracks) in &self.world.resource::<KnowledgeBases>().0 {
            for track in tracks {
                let mut track = track.clone();
                track.uncertainty_m = track.uncertainty_m.max(100.0);
                runtime.import_track(*owner, track, tick);
            }
        }
        self.world.insert_resource(runtime);
        Ok(())
    }
    pub fn sensing_configuration(&self) -> Option<&Configuration> {
        self.world
            .get_resource::<Runtime>()
            .map(|s| &s.configuration)
    }
    pub fn environment(&self) -> Option<&Environment> {
        self.world.get_resource::<Runtime>().map(|s| &s.environment)
    }
    pub fn schedule_weather(
        &mut self,
        expected: u64,
        baseline: Weather,
        regions: Vec<WeatherRegion>,
    ) -> Result<(), String> {
        let tick = self.tick() + 1;
        self.world
            .get_resource_mut::<Runtime>()
            .ok_or("sensing is not configured")?
            .environment
            .schedule(expected, tick, baseline, regions)
    }
    pub fn receive_bearing_report(&mut self, owner: Uuid, bearing: Bearing) -> bool {
        let tick = self.tick();
        let Some(mut runtime) = self.world.get_resource_mut::<Runtime>() else {
            return false;
        };
        let accepted = runtime.receive_bearing(owner, bearing, tick);
        if accepted {
            self.sync_sensing_knowledge(owner);
        }
        accepted
    }
    pub(crate) fn sync_sensing_knowledge(&mut self, owner: Uuid) {
        let tracks = self
            .world
            .resource::<Runtime>()
            .tracks_for(owner, self.tick());
        self.world
            .resource_mut::<KnowledgeBases>()
            .0
            .insert(owner, tracks);
    }
    pub fn sensing_terrain_pin(&self) -> Option<&Manifest> {
        self.world
            .get_resource::<Runtime>()
            .and_then(|s| s.terrain_pin.as_ref())
    }
    pub fn refresh_sensing_knowledge(&mut self) {
        let Some(runtime) = self.world.get_resource::<Runtime>() else {
            return;
        };
        let owners: Vec<_> = runtime.tracks.keys().copied().collect();
        for owner in owners {
            self.sync_sensing_knowledge(owner);
        }
    }
}
impl Simulation {
    pub fn rebase_ground_units(&mut self) -> Result<(), String> {
        let terrain = self
            .world
            .get_resource::<Runtime>()
            .and_then(|r| r.terrain.clone());
        if let Some(terrain) = terrain {
            let mut query = self
                .world
                .query::<(&DomainKind, &mut GeoPose, &mut GeodesicMotionState)>();
            for (domain, mut position, mut motion) in query.iter_mut(&mut self.world) {
                if domain.0 == Domain::Land {
                    position.altitude_m = terrain
                        .height(position.latitude_deg, position.longitude_deg)
                        .ok_or("ground unit lies outside terrain coverage")?;
                    motion.origin = *position;
                }
            }
        }
        Ok(())
    }
}
pub fn regional_terrain_ready() -> bool {
    load_terrain(None).is_ok()
}

impl Runtime {
    fn import_track(&mut self, owner: Uuid, mut track: Track, tick: u64) {
        if tick.saturating_sub(track.observed_tick) > self.configuration.model.track_expiry_ticks {
            return;
        }
        let covariance = diagonal((track.uncertainty_m / 2.448).powi(2));
        track.evidence = Some(Evidence {
            hypotheses: vec![ClassificationHypothesis {
                domain: track.observed_domain,
                side: track.target_side,
                probability: track.identity_confidence,
            }],
            estimate_tick: track.observed_tick,
            modalities: vec![Modality::Radar],
            covariance,
            observations: BTreeSet::from([track.track_id]),
            velocity: Velocity::default(),
        });
        self.tracks.entry(owner).or_default().push(Estimate {
            origin: track.position,
            state: [0.0; 4],
            covariance,
            predicted_tick: track.observed_tick,
            identity_tick: track.observed_tick,
            local_tick: None,
            track,
        });
    }
}

impl Simulation {
    pub fn local_weather_report(&mut self, owner: Uuid) -> Option<WeatherObservation> {
        let mut query = self.world.query::<(&SimEntityId, &GeoPose)>();
        let position = query
            .iter(&self.world)
            .find(|(id, _)| id.0 == owner)
            .map(|(_, pose)| *pose)?;
        let runtime = self.world.get_resource::<Runtime>()?;
        Some(WeatherObservation {
            source: owner,
            position,
            conditions: runtime.environment.at(position, self.tick()),
            observed_tick: self.tick(),
            received_tick: self.tick(),
        })
    }
    pub fn receive_weather_report(&mut self, owner: Uuid, mut report: WeatherObservation) -> bool {
        let tick = self.tick();
        if report.observed_tick > tick
            || !geo_pose_is_finite(report.position)
            || report.conditions.validate().is_err()
        {
            return false;
        }
        let Some(mut runtime) = self.world.get_resource_mut::<Runtime>() else {
            return false;
        };
        let reports = runtime.weather_reports.entry(owner).or_default();
        if reports
            .get(&report.source)
            .is_some_and(|old| old.observed_tick >= report.observed_tick)
        {
            return false;
        }
        report.received_tick = tick;
        reports.insert(report.source, report);
        true
    }
}

impl Simulation {
    /// Adapter for the pinned c3mesh v1 snapshot wire schema. Preparing a replacement
    /// preserves queues, packet IDs, histories, channel states and scheduled events.
    /// Only future packet admissions/serialization use the upgraded training limits.
    fn upgrade_training_radio(&mut self, radio: &RadioUpgrade) -> Result<(), String> {
        radio.validate()?;
        let mut value = serde_json::to_value(self.communications.simulator.snapshot())
            .map_err(|e| e.to_string())?;
        if value["schema_version"] != 1 {
            return Err("unsupported c3mesh radio migration schema".into());
        }
        let ids: BTreeSet<_> = self
            .communications
            .links
            .iter()
            .map(|link| link.channel_id.to_string())
            .collect();
        let channels = value["configuration"]["channels"]
            .as_array_mut()
            .ok_or("invalid c3mesh configuration")?;
        for channel in channels {
            if channel["id"].as_str().is_some_and(|id| ids.contains(id)) {
                channel["bit_rate_bps"] = serde_json::json!(radio.bit_rate_bps);
            }
        }
        for id in ids {
            let runtime = value["channels"]
                .get_mut(&id)
                .ok_or("missing c3mesh runtime channel")?;
            runtime["nominal_bit_rate_bps"] = serde_json::json!(radio.bit_rate_bps);
            let options = value["options"]["channels"]
                .get_mut(&id)
                .ok_or("missing c3mesh channel options")?;
            options["mtu_bytes"] = serde_json::json!(radio.mtu_bytes);
            options["queue"]["max_packets"] = serde_json::json!(radio.queue_packets);
            options["queue"]["max_bytes"] = serde_json::json!(radio.queue_bytes);
        }
        let snapshot = serde_json::from_value(value).map_err(|e| e.to_string())?;
        let prepared = NetworkSimulator::restore(snapshot)?;
        self.communications.simulator = prepared;
        Ok(())
    }
}
