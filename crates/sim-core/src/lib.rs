//! Deterministic, server-authoritative primitives for World At War.

use std::collections::{BTreeMap, VecDeque};

use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
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

#[derive(Component, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CommunicationNode {
    pub range_m: f64,
    pub operational: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerIntent {
    pub intent_id: Uuid,
    pub issuer_role: Uuid,
    pub target: Uuid,
    pub kind: OrderKind,
    pub requested_tick: u64,
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
    pub communication: Option<CommunicationNode>,
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
pub struct PendingIntents(pub VecDeque<PlayerIntent>);

#[derive(Resource, Debug, Default)]
pub struct OrderResults(pub Vec<OrderResult>);

pub struct Simulation {
    world: World,
    schedule: Schedule,
}

impl Simulation {
    pub fn new() -> Self {
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
        Self { world, schedule }
    }

    pub fn spawn_platform(&mut self, platform: PlatformSpawn) {
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
        if let Some(communication) = platform.communication {
            entity.insert(communication);
        }
    }

    pub fn queue_intent(&mut self, intent: PlayerIntent) {
        self.world
            .resource_mut::<PendingIntents>()
            .0
            .push_back(intent);
    }

    pub fn step(&mut self) {
        self.schedule.run(&mut self.world);
    }

    pub fn tick(&self) -> u64 {
        self.world.resource::<SimClock>().tick
    }

    pub fn drain_order_results(&mut self) -> Vec<OrderResult> {
        std::mem::take(&mut self.world.resource_mut::<OrderResults>().0)
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
                });
            }
        }
        RoleProjection {
            tick: self.tick(),
            own_units,
            tracks,
        }
    }
}

impl Default for Simulation {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisibleUnit {
    pub id: Uuid,
    pub name: String,
    pub domain: Domain,
    pub position: GeoPose,
    pub sidc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleProjection {
    pub tick: u64,
    pub own_units: Vec<VisibleUnit>,
    pub tracks: Vec<Track>,
}

fn advance_clock(mut clock: ResMut<SimClock>) {
    clock.tick += 1;
}

fn apply_orders(
    clock: Res<SimClock>,
    mut pending: ResMut<PendingIntents>,
    mut results: ResMut<OrderResults>,
    mut units: Query<(&SimEntityId, &AuthorityNode, &mut Velocity)>,
) {
    let mut deferred = VecDeque::new();
    while let Some(intent) = pending.0.pop_front() {
        if intent.requested_tick > clock.tick {
            deferred.push_back(intent);
            continue;
        }
        let Some((_, authority, mut velocity)) =
            units.iter_mut().find(|(id, _, _)| id.0 == intent.target)
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

fn move_platforms(mut units: Query<(&mut GeoPose, &Velocity)>) {
    for (mut pose, velocity) in &mut units {
        pose.latitude_deg += velocity.north_mps * TICK_SECONDS as f64 / 111_320.0;
        let longitude_scale = 111_320.0 * pose.latitude_deg.to_radians().cos().abs().max(0.01);
        pose.longitude_deg += velocity.east_mps * TICK_SECONDS as f64 / longitude_scale;
        pose.altitude_m = (pose.altitude_m + velocity.climb_mps * TICK_SECONDS as f64).max(0.0);
    }
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

    #[test]
    fn detects_unit_when_inside_sensor_range() {
        let mut sim = Simulation::new();
        let blue = Uuid::new_v4();
        sim.spawn_platform(PlatformSpawn {
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
            communication: None,
            sidc: "100301000011010000000000000000".into(),
        });
        sim.spawn_platform(PlatformSpawn {
            id: Uuid::new_v4(),
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
            communication: None,
            sidc: "100601000011010000000000000000".into(),
        });
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
        let mut sim = Simulation::new();
        let blue = Uuid::new_v4();
        sim.spawn_platform(PlatformSpawn {
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
            communication: None,
            sidc: "100310000012110000000000000000".into(),
        });
        sim.spawn_platform(PlatformSpawn {
            id: Uuid::new_v4(),
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
            communication: None,
            sidc: "100610000012110000000000000000".into(),
        });
        assert_eq!(sim.projection_for(blue, Side::Blue).own_units.len(), 1);
        assert!(sim.projection_for(blue, Side::Blue).tracks.is_empty());
    }
}
