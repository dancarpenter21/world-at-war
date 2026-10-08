//! Read-only simulation inspection. Never merge this data into a role knowledge base.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthUnit {
    pub side: Side,
    #[serde(flatten)]
    pub state: VisibleUnit,
    pub fuel: f64,
    pub ammunition: u32,
    pub destroyed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthWeapon {
    pub id: String,
    pub kind: String,
    pub side: Option<Side>,
    pub launcher_id: Option<Uuid>,
    pub position: Option<GeoPose>,
    pub aim_point: Option<GeoPose>,
    pub impact_tick: Option<u64>,
    pub phase: String,
    pub provider_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthProjection {
    pub tick: u64,
    pub units: Vec<TruthUnit>,
    pub weapons: Vec<TruthWeapon>,
    pub jamming_regions: Vec<JammingRegion>,
    pub communication_links: Vec<CommunicationLinkStatus>,
}

impl Simulation {
    pub fn truth_projection(&mut self) -> TruthProjection {
        let tick = self.tick();
        let at = self.network_time();
        let mut query = self.world.query::<(
            &SimEntityId,
            &PlatformName,
            &Ownership,
            &DomainKind,
            &GeoPose,
            &PlatformSidc,
            &Velocity,
            Option<&CyclicFlightPathState>,
            &operations::CombatState,
        )>();
        let mut units: Vec<_> = query
            .iter(&self.world)
            .map(
                |(id, name, side, domain, pose, sidc, velocity, path, state)| {
                    let hp = self.world.resource::<CombatState>().hit_points(id.0);
                    TruthUnit {
                        side: side.0,
                        state: VisibleUnit {
                            id: id.0,
                            name: name.0.clone(),
                            domain: domain.0,
                            position: *pose,
                            velocity: *velocity,
                            following_flight_path: path.is_some_and(|p| p.active),
                            sidc: sidc.0.clone(),
                            receiver_jammed: self.entity_receiver_jammed(id.0, at),
                            observed_tick: tick,
                            received_tick: tick,
                            weapon: self.world.resource::<CombatState>().weapon_status(id.0),
                            hit_points: hp,
                        },
                        fuel: state.profile.fuel_seconds,
                        ammunition: self
                            .world
                            .resource::<CombatState>()
                            .weapon_status(id.0)
                            .map_or(state.profile.ammunition, |w| w.ammunition),
                        destroyed: hp == Some(0) || state.destroyed,
                    }
                },
            )
            .collect();
        units.sort_by_key(|u| u.state.id);
        let system = self.world.resource::<iftu::WeaponSystem>();
        let mut weapons = system.truth_weapons();
        weapons.extend(self.world.resource::<CombatState>().truth_weapons());
        weapons.extend(
            self.world
                .resource::<operations::Operations>()
                .truth_weapons(),
        );
        weapons.sort_by(|a, b| a.id.cmp(&b.id));
        let temporary = system.truth_links();
        let mut links = BTreeMap::new();
        for link in self.communications.links.iter().chain(temporary.iter()) {
            let Ok(metrics) = self.communications.simulator.transmission_metrics_at(
                link.channel_id.clone(),
                link.source_device_id.clone(),
                at,
            ) else {
                continue;
            };
            let Ok(queue) = self
                .communications
                .simulator
                .channel_queue_metrics(link.channel_id.clone())
            else {
                continue;
            };
            links.insert(
                link.id.clone(),
                CommunicationLinkStatus {
                    id: link.id.clone(),
                    from_entity_id: link.from_entity_id,
                    to_entity_id: link.to_entity_id,
                    available: metrics.available,
                    jammed: metrics.jammed,
                    effective_bit_rate_bps: metrics.effective_bit_rate_bps,
                    queued_packets: queue.packets_0_to_1 + queue.packets_1_to_0,
                    queued_bytes: queue.bytes_0_to_1 + queue.bytes_1_to_0,
                },
            );
        }
        TruthProjection {
            tick,
            units,
            weapons,
            jamming_regions: self
                .communications
                .jamming_regions
                .iter()
                .filter(|r| r.active_at(self.radio_tick()))
                .cloned()
                .collect(),
            communication_links: links.into_values().collect(),
        }
    }
}
