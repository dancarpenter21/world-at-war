//! Opt-in binary radio geometry. Obstruction is separate from receiver jamming.
use super::*;
use sim_geo::terrain::Visibility;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub version: u32,
    pub terrain: bool,
    pub source_antenna_height_m: f64,
    pub destination_antenna_height_m: f64,
    pub max_range_m: f64,
}
impl Configuration {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !self.max_range_m.is_finite()
            || self.max_range_m <= 0.0
            || [
                self.source_antenna_height_m,
                self.destination_antenna_height_m,
            ]
            .iter()
            .any(|h| !h.is_finite() || *h < 0.0)
        {
            return Err("invalid radio propagation configuration".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeometryStatus {
    Clear,
    EarthBlocked,
    TerrainBlocked,
    TerrainUnknown,
    OutOfRange,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Runtime {
    baseline: BTreeMap<ChannelId, CommunicationChannelState>,
    statuses: BTreeMap<String, GeometryStatus>,
    #[serde(skip)]
    cache: BTreeMap<String, CacheEntry>,
}
#[derive(Clone)]
struct CacheEntry {
    from: GeoPose,
    to: GeoPose,
    terrain_digest: Option<String>,
    config: Configuration,
    status: GeometryStatus,
}
impl Runtime {
    pub(crate) fn new(config: &CommunicationsConfig) -> Option<Self> {
        let baseline: BTreeMap<_, _> = config
            .links
            .iter()
            .filter(|l| l.propagation.is_some())
            .filter_map(|l| {
                config
                    .network
                    .channels
                    .iter()
                    .find(|c| c.id == l.channel_id)
                    .map(|c| (c.id.clone(), c.state))
            })
            .collect();
        (!baseline.is_empty()).then_some(Self {
            baseline,
            statuses: BTreeMap::new(),
            cache: BTreeMap::new(),
        })
    }
}
impl Simulation {
    pub fn uses_radio_propagation(&self) -> bool {
        self.communications.propagation.is_some()
    }

    pub(crate) fn radio_geometry_status(&self, id: &str) -> Option<GeometryStatus> {
        self.communications
            .propagation
            .as_ref()?
            .statuses
            .get(id)
            .copied()
    }

    /// Initialize after scenario sensing and ground positions are configured.
    /// Snapshots retain their last radio-boundary state; this does not advance time.
    pub fn initialize_radio_geometry(&mut self) -> Result<(), String> {
        self.update_radio_geometry().map_err(|e| e.to_string())
    }

    pub(super) fn sync_radio_geometry(&mut self) -> Result<(), c3mesh::SimulationError> {
        if !self.uses_radio_propagation() {
            return Ok(());
        }
        // Finish traffic under the previous channel state before changing the state
        // at this boundary. c3mesh retains the semantics of already-started packets.
        let events = self.advance_network()?;
        self.pending_network_events.extend(events);
        self.update_radio_geometry()
    }

    fn update_radio_geometry(&mut self) -> Result<(), c3mesh::SimulationError> {
        let Some(mut runtime) = self.communications.propagation.take() else {
            return Ok(());
        };
        let result = (|| {
            let mut query = self.world.query::<(&SimEntityId, &GeoPose)>();
            let positions: BTreeMap<_, _> = query
                .iter(&self.world)
                .map(|(id, pose)| (id.0, *pose))
                .collect();
            let terrain = self
                .world
                .get_resource::<sensing::Runtime>()
                .and_then(|r| r.terrain.clone());
            let digest = terrain.as_ref().map(|t| t.manifest.digest());
            for link in &self.communications.links {
                let Some(config) = &link.propagation else {
                    continue;
                };
                let (Some(from), Some(to)) = (
                    positions.get(&link.from_entity_id),
                    positions.get(&link.to_entity_id),
                ) else {
                    continue;
                };
                let mut from = *from;
                let mut to = *to;
                from.altitude_m += config.source_antenna_height_m;
                to.altitude_m += config.destination_antenna_height_m;
                let cached = runtime.cache.get(&link.id).filter(|c| {
                    c.from == from
                        && c.to == to
                        && c.config == *config
                        && c.terrain_digest == digest
                });
                let status = if let Some(cached) = cached {
                    cached.status
                } else {
                    let status = if great_circle_distance_m(from, to) > config.max_range_m {
                        GeometryStatus::OutOfRange
                    } else if !geodesy::has_geometric_line_of_sight(from, to) {
                        GeometryStatus::EarthBlocked
                    } else if config.terrain {
                        match terrain.as_ref().map(|t| {
                            t.visibility(
                                [from.latitude_deg, from.longitude_deg, from.altitude_m],
                                [to.latitude_deg, to.longitude_deg, to.altitude_m],
                            )
                        }) {
                            Some(Visibility::Clear) => GeometryStatus::Clear,
                            Some(Visibility::Blocked) => GeometryStatus::TerrainBlocked,
                            _ => GeometryStatus::TerrainUnknown,
                        }
                    } else {
                        GeometryStatus::Clear
                    };
                    runtime.cache.insert(
                        link.id.clone(),
                        CacheEntry {
                            from,
                            to,
                            terrain_digest: digest.clone(),
                            config: config.clone(),
                            status,
                        },
                    );
                    status
                };
                runtime.statuses.insert(link.id.clone(), status);
                let state = if status == GeometryStatus::Clear {
                    runtime.baseline[&link.channel_id]
                } else {
                    CommunicationChannelState::Severed
                };
                self.communications
                    .simulator
                    .set_channel_state(link.channel_id.clone(), state)?;
            }
            Ok(())
        })();
        self.communications.propagation = Some(runtime);
        result
    }
}
