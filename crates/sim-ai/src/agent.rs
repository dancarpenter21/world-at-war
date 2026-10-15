//! Provider-neutral, role-scoped decision protocol. No simulation truth access.
use serde::{Deserialize, Serialize};
use sim_core::{GeoPose, OrderKind, RoleProjection};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    pub id: Uuid,
    pub role_id: Uuid,
    pub name: String,
    pub priority: u32,
    pub center: GeoPose,
    pub home: GeoPose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_facility_id: Option<Uuid>,
    pub radius_m: f64,
    pub speed_mps: f64,
    pub minimum_identification: f32,
    pub max_track_age_ticks: u64,
    pub return_below_fuel_fraction: f64,
    pub return_below_hit_points: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentObservation {
    pub protocol_version: u32,
    pub observation_id: Uuid,
    pub assignment_generation: u64,
    pub role_id: Uuid,
    pub side: sim_core::Side,
    pub controlled_units: Vec<Uuid>,
    #[serde(default)]
    pub goal_assignments: std::collections::BTreeMap<Uuid, Uuid>,
    pub goals: Vec<Goal>,
    pub projection: RoleProjection,
    pub available_actions: Vec<String>,
    pub authority_requests: Vec<serde_json::Value>,
    pub receipts: Vec<serde_json::Value>,
    pub deadline_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentAction {
    Order { unit_id: Uuid, kind: OrderKind },
    SelectGoal { goal_id: Uuid, unit_ids: Vec<Uuid> },
    AuthorityDecision { request_id: Uuid, approve: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentDecision {
    pub protocol_version: u32,
    pub observation_id: Uuid,
    pub assignment_generation: u64,
    pub decision_id: Uuid,
    pub actions: Vec<AgentAction>,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub model: Option<String>,
}

/// Distances use only supplied knowledge. The small exercise uses a local tangent plane.
pub fn displacement(from: GeoPose, to: GeoPose) -> (f64, f64) {
    let north = (to.latitude_deg - from.latitude_deg).to_radians() * 6_371_000.0;
    let longitude = (to.longitude_deg - from.longitude_deg + 180.0).rem_euclid(360.0) - 180.0;
    let east = longitude.to_radians() * 6_371_000.0 * from.latitude_deg.to_radians().cos();
    (north, east)
}

/// Stable allocation: nearest eligible aircraft per known threat; remaining aircraft defend.
pub fn defensive_actions(
    observation: &AgentObservation,
    assignments: &std::collections::BTreeMap<Uuid, Uuid>,
    seed: u64,
) -> Vec<AgentAction> {
    use sim_core::{Domain, OrderKind};
    let p = &observation.projection;
    let mut units: Vec<_> = p
        .own_units
        .iter()
        .filter(|u| {
            observation.controlled_units.contains(&u.id)
                && u.domain == Domain::Air
                && u.hit_points != Some(0)
        })
        .collect();
    units.sort_by_key(|u| u.id.as_u128() ^ u128::from(seed));
    let mut goals: Vec<_> = observation
        .goals
        .iter()
        .filter(|g| g.role_id == observation.role_id)
        .collect();
    goals.sort_by_key(|g| (std::cmp::Reverse(g.priority), g.id));
    let goal_for = |unit: &sim_core::VisibleUnit| {
        assignments
            .get(&unit.id)
            .and_then(|id| goals.iter().find(|g| g.id == *id).copied())
            .or_else(|| goals.first().copied())
    };
    let retreating = |unit: &sim_core::VisibleUnit, goal: &Goal| {
        unit.hit_points
            .is_some_and(|hp| hp <= goal.return_below_hit_points)
            || unit.weapon.as_ref().is_some_and(|w| w.ammunition == 0)
            || p.logistics
                .as_ref()
                .and_then(|l| l.units.iter().find(|r| r.unit_id == unit.id))
                .is_some_and(|r| {
                    r.capacity_kg > 0.0
                        && r.fuel_kg / r.capacity_kg <= goal.return_below_fuel_fraction
                })
    };
    let eligible = |unit: &sim_core::VisibleUnit| {
        p.logistics.as_ref().is_none_or(|l| {
            l.units
                .iter()
                .any(|r| r.unit_id == unit.id && r.phase == sim_core::logistics::Phase::Deployed)
        })
    };
    let mut allocated = std::collections::BTreeMap::new();
    let mut used_tracks = std::collections::BTreeSet::new();
    for goal in &goals {
        let mut tracks: Vec<_> = p
            .tracks
            .iter()
            .filter(|t| {
                t.target_side.is_some_and(|s| s != observation.side)
                    && t.assessed_destroyed != Some(true)
                    && p.tick.saturating_sub(t.observed_tick) <= goal.max_track_age_ticks
                    && t.identity_confidence >= goal.minimum_identification
                    && {
                        let (n, e) = displacement(goal.center, t.position);
                        n.hypot(e) <= goal.radius_m
                    }
            })
            .collect();
        tracks.sort_by(|a, b| {
            let (an, ae) = displacement(goal.center, a.position);
            let (bn, be) = displacement(goal.center, b.position);
            an.hypot(ae)
                .total_cmp(&bn.hypot(be))
                .then(a.track_id.cmp(&b.track_id))
        });
        for track in tracks {
            if used_tracks.contains(&track.track_id) {
                continue;
            }
            let selected = units
                .iter()
                .copied()
                .filter(|u| {
                    !allocated.contains_key(&u.id)
                        && eligible(u)
                        && !retreating(u, goal)
                        && goal_for(u).is_some_and(|g| g.id == goal.id)
                })
                .min_by(|a, b| {
                    let (an, ae) = displacement(a.position, track.position);
                    let (bn, be) = displacement(b.position, track.position);
                    an.hypot(ae).total_cmp(&bn.hypot(be)).then(
                        (a.id.as_u128() ^ u128::from(seed))
                            .cmp(&(b.id.as_u128() ^ u128::from(seed))),
                    )
                });
            if let Some(unit) = selected {
                allocated.insert(unit.id, track);
                used_tracks.insert(track.track_id);
            }
        }
    }
    let mut actions = Vec::new();
    for unit in units.iter().copied() {
        let Some(goal) = goal_for(unit) else {
            continue;
        };
        if !eligible(unit) {
            continue;
        }
        let resource = p
            .logistics
            .as_ref()
            .and_then(|l| l.units.iter().find(|r| r.unit_id == unit.id));
        let retreat = retreating(unit, goal);
        if retreat && resource.is_some() {
            if let Some(facility_id) = goal.home_facility_id {
                actions.push(AgentAction::Order {
                    unit_id: unit.id,
                    kind: OrderKind::Logistics {
                        command: sim_core::logistics::Command::Return { facility_id },
                    },
                });
                continue;
            }
        }
        let track = allocated.get(&unit.id).copied();
        let target = if retreat {
            goal.home
        } else if let Some(track) = track {
            let (n, e) = displacement(unit.position, track.position);
            if unit.weapon.as_ref().is_some_and(|w| {
                w.ammunition > 0
                    && n.hypot(e) <= w.range_m
                    && p.tick.saturating_sub(track.observed_tick) <= w.max_track_age_ticks
            }) {
                actions.push(AgentAction::Order {
                    unit_id: unit.id,
                    kind: OrderKind::Engage {
                        track_id: track.track_id,
                    },
                });
                continue;
            }
            track.position
        } else {
            goal.center
        };
        let (n, e) = displacement(unit.position, target);
        let distance = n.hypot(e);
        let speed = if distance < 250.0 {
            0.0
        } else {
            goal.speed_mps.min(distance / 5.0)
        };
        let (north_mps, east_mps) = if speed == 0.0 {
            (0.0, 0.0)
        } else {
            (n / distance * speed, e / distance * speed)
        };
        if unit.following_flight_path
            || (unit.velocity.north_mps - north_mps).abs() > 1.0
            || (unit.velocity.east_mps - east_mps).abs() > 1.0
        {
            actions.push(AgentAction::Order {
                unit_id: unit.id,
                kind: OrderKind::Move {
                    north_mps,
                    east_mps,
                },
            });
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn observation() -> AgentObservation {
        let p = crate::tests::projection();
        let center = p.own_units[0].position;
        AgentObservation {
            protocol_version: 1,
            observation_id: Uuid::nil(),
            assignment_generation: 0,
            role_id: Uuid::from_u128(2),
            side: sim_core::Side::Red,
            controlled_units: vec![Uuid::from_u128(1)],
            goal_assignments: Default::default(),
            goals: vec![Goal {
                id: Uuid::from_u128(4),
                role_id: Uuid::from_u128(2),
                name: "Defense".into(),
                priority: 100,
                center,
                home: center,
                home_facility_id: None,
                radius_m: 50000.0,
                speed_mps: 180.0,
                minimum_identification: 0.4,
                max_track_age_ticks: 15,
                return_below_fuel_fraction: 0.2,
                return_below_hit_points: 20,
            }],
            projection: p,
            available_actions: vec![],
            authority_requests: vec![],
            receipts: vec![],
            deadline_unix_ms: 0,
        }
    }
    #[test]
    fn nearest_aircraft_engages_each_contact_once() {
        let mut obs = observation();
        crate::tests::add_contact(&mut obs.projection);
        obs.projection.tracks[0].position.latitude_deg += 0.02;
        obs.projection.own_units[0].weapon = Some(sim_core::combat::WeaponStatus {
            ammunition: 2,
            range_m: 10000.0,
            max_track_age_ticks: 15,
        });
        let mut other = obs.projection.own_units[0].clone();
        other.id = Uuid::from_u128(5);
        other.position.latitude_deg += 0.019;
        obs.controlled_units.push(other.id);
        obs.projection.own_units.push(other);
        let actions = defensive_actions(&obs, &BTreeMap::new(), 0);
        let engaged: Vec<_> = actions
            .iter()
            .filter_map(|a| {
                if let AgentAction::Order {
                    unit_id,
                    kind: OrderKind::Engage { .. },
                } = a
                {
                    Some(*unit_id)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(engaged, vec![Uuid::from_u128(5)]);
    }
    #[test]
    fn stale_unidentified_friendly_and_destroyed_tracks_do_not_cue_interception() {
        let mut obs = observation();
        let baseline = defensive_actions(&obs, &BTreeMap::new(), 7);
        crate::tests::add_contact(&mut obs.projection);
        obs.projection.tracks[0].position.latitude_deg += 0.1;
        for condition in 0..4 {
            let mut changed = obs.clone();
            match condition {
                0 => changed.projection.tracks[0].observed_tick = 0,
                1 => changed.projection.tracks[0].identity_confidence = 0.1,
                2 => changed.projection.tracks[0].target_side = Some(sim_core::Side::Red),
                _ => changed.projection.tracks[0].assessed_destroyed = Some(true),
            }
            if condition == 0 {
                changed.projection.tick = 30;
            }
            assert_eq!(defensive_actions(&changed, &BTreeMap::new(), 7), baseline);
        }
    }
    #[test]
    fn depleted_aircraft_return_home_without_chasing_and_unknown_logistics_do_not_imply_readiness()
    {
        let mut obs = observation();
        crate::tests::add_contact(&mut obs.projection);
        obs.projection.own_units[0].position.latitude_deg += 0.1;
        obs.projection.own_units[0].weapon = Some(sim_core::combat::WeaponStatus {
            ammunition: 0,
            range_m: 10000.0,
            max_track_age_ticks: 15,
        });
        let actions = defensive_actions(&obs, &BTreeMap::new(), 1);
        assert!(
            matches!(actions.first(),Some(AgentAction::Order{kind:OrderKind::Move{north_mps,..},..}) if *north_mps<0.0)
        );
        obs.projection.logistics = Some(Default::default());
        assert!(defensive_actions(&obs, &BTreeMap::new(), 1).is_empty());
    }
    #[test]
    fn local_geometry_wraps_the_antimeridian() {
        let (n, e) = displacement(
            sim_core::GeoPose {
                latitude_deg: 0.0,
                longitude_deg: 179.9,
                altitude_m: 0.0,
            },
            sim_core::GeoPose {
                latitude_deg: 0.0,
                longitude_deg: -179.9,
                altitude_m: 0.0,
            },
        );
        assert_eq!(n, 0.0);
        assert!((20000.0..23000.0).contains(&e));
    }
}
