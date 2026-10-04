//! Basic Red planner. It accepts a role projection and cannot inspect ECS truth.

use sim_core::{Domain, OrderKind, PlayerIntent, RoleProjection};
use uuid::Uuid;

pub fn choose_patrol_intent(
    role: Uuid,
    controlled_unit: Uuid,
    projection: &RoleProjection,
) -> Option<PlayerIntent> {
    let unit = projection
        .own_units
        .iter()
        .find(|unit| unit.id == controlled_unit)?;
    if unit.domain != Domain::Air {
        return None;
    }
    let east_mps = if projection.tracks.is_empty() {
        -180.0
    } else {
        -120.0
    };
    if !unit.following_flight_path
        && unit.velocity.north_mps.abs() < 1e-6
        && (unit.velocity.east_mps - east_mps).abs() < 1e-6
    {
        return None;
    }
    Some(PlayerIntent {
        intent_id: Uuid::new_v4(),
        issuer_role: role,
        target: unit.id,
        kind: OrderKind::Move {
            north_mps: 0.0,
            east_mps,
        },
        requested_tick: projection.tick.saturating_add(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::{GeoPose, Side, Track, Velocity, VisibleUnit};

    fn projection() -> RoleProjection {
        RoleProjection {
            iftu: Default::default(),
            tick: 10,
            own_units: vec![VisibleUnit {
                observed_tick: 0,
                received_tick: 0,
                id: Uuid::from_u128(1),
                name: "Patrol aircraft".into(),
                domain: Domain::Air,
                position: GeoPose {
                    latitude_deg: 0.0,
                    longitude_deg: 0.0,
                    altitude_m: 8_000.0,
                },
                velocity: Velocity {
                    north_mps: 0.0,
                    east_mps: 180.0,
                    climb_mps: 0.0,
                },
                following_flight_path: false,
                sidc: String::new(),
                receiver_jammed: false,
                weapon: None,
                hit_points: None,
            }],
            tracks: vec![],
            jamming_regions: vec![],
            communication_links: vec![],
            combat: None,
        }
    }

    fn plan(projection: &RoleProjection) -> Option<PlayerIntent> {
        choose_patrol_intent(Uuid::from_u128(2), Uuid::from_u128(1), projection)
    }

    fn add_contact(projection: &mut RoleProjection) {
        projection.tracks.push(Track {
            uncertainty_m: 100.0,
            assessed_destroyed: None,
            observed_domain: None,
            track_id: Uuid::from_u128(3),
            target_side: Some(Side::Blue),
            position: projection.own_units[0].position,
            identity_confidence: 0.5,
            observed_tick: 9,
            received_tick: 10,
            observed_sidc: String::new(),
        });
    }

    #[test]
    fn commands_a_changed_airborne_patrol_and_skips_an_already_active_order() {
        let mut observed = projection();
        let intent = plan(&observed).unwrap();
        assert_eq!(intent.target, Uuid::from_u128(1));
        assert_eq!(intent.issuer_role, Uuid::from_u128(2));
        assert_eq!(intent.requested_tick, 11);
        assert_eq!(
            intent.kind,
            OrderKind::Move {
                north_mps: 0.0,
                east_mps: -180.0
            }
        );
        observed.own_units[0].velocity.east_mps = -180.0;
        assert!(plan(&observed).is_none());
    }

    #[test]
    fn responds_once_when_available_contact_information_changes() {
        let mut observed = projection();
        observed.own_units[0].velocity.east_mps = -180.0;
        add_contact(&mut observed);
        assert_eq!(
            plan(&observed).unwrap().kind,
            OrderKind::Move {
                north_mps: 0.0,
                east_mps: -120.0
            }
        );
        observed.own_units[0].velocity.east_mps = -120.0;
        assert!(plan(&observed).is_none());
        observed.tracks.clear();
        assert_eq!(
            plan(&observed).unwrap().kind,
            OrderKind::Move {
                north_mps: 0.0,
                east_mps: -180.0
            }
        );
    }

    #[test]
    fn does_not_patrol_land_units_or_units_outside_the_projection() {
        let mut observed = projection();
        observed.own_units[0].domain = Domain::Land;
        assert!(plan(&observed).is_none());
        observed.own_units.clear();
        assert!(plan(&observed).is_none());
    }

    #[test]
    fn replaces_an_authored_path_even_when_its_stored_velocity_matches() {
        let mut observed = projection();
        observed.own_units[0].velocity.east_mps = -180.0;
        observed.own_units[0].following_flight_path = true;
        assert!(plan(&observed).is_some());
    }
}
