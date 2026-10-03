//! Spherical geometry for movement, authored paths, and geometric sensors.
//!
//! Coordinates use the existing mean-radius Earth, not an ellipsoid or terrain model.
//! Atmospheric refraction and platform antenna offsets are deliberately not included.

use crate::GeoPose;

pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
const HORIZON_TOLERANCE_RAD: f64 = 1.0e-10;

/// Ground arc distance. Altitude does not affect geographic regions such as jamming areas.
pub fn surface_distance_m(a: GeoPose, b: GeoPose) -> f64 {
    EARTH_RADIUS_M * central_angle_rad(a, b)
}

/// Straight-line distance between two positions, including their altitude difference.
pub fn slant_distance_m(a: GeoPose, b: GeoPose) -> f64 {
    if !crate::geo_pose_is_finite(a) || !crate::geo_pose_is_finite(b) {
        return f64::NAN;
    }
    let a_radius = EARTH_RADIUS_M + a.altitude_m;
    let b_radius = EARTH_RADIUS_M + b.altitude_m;
    if a_radius < 0.0 || b_radius < 0.0 {
        return f64::NAN;
    }
    // This rearrangement of the cosine rule avoids subtracting nearly equal squares.
    let horizontal_chord =
        2.0 * a_radius.sqrt() * b_radius.sqrt() * (central_angle_rad(a, b) / 2.0).sin();
    horizontal_chord.hypot(a.altitude_m - b.altitude_m)
}

/// Whether the segment between two positions clears the spherical Earth.
///
/// Each endpoint contributes its own horizon angle. Negative-altitude positions cannot
/// participate in this geometric sensor model; underwater sensing needs a separate modality.
/// Tangency counts as visible, with an angular tolerance under one millimeter at Earth's radius.
pub fn has_geometric_line_of_sight(a: GeoPose, b: GeoPose) -> bool {
    if !crate::geo_pose_is_finite(a)
        || !crate::geo_pose_is_finite(b)
        || a.altitude_m < 0.0
        || b.altitude_m < 0.0
    {
        return false;
    }
    central_angle_rad(a, b)
        <= horizon_angle_rad(a.altitude_m) + horizon_angle_rad(b.altitude_m) + HORIZON_TOLERANCE_RAD
}

/// Move along a great-circle arc using north/east displacement in the initial local frame.
/// Altitude is updated independently; callers apply their domain's altitude limits.
/// The initial tangent frame also defines consistent motion from either exact pole.
pub fn advance_position(
    position: GeoPose,
    north_m: f64,
    east_m: f64,
    climb_m: f64,
) -> Option<GeoPose> {
    if !crate::geo_pose_is_finite(position)
        || !north_m.is_finite()
        || !east_m.is_finite()
        || !climb_m.is_finite()
    {
        return None;
    }
    let distance = north_m.hypot(east_m);
    let radius = EARTH_RADIUS_M + position.altitude_m;
    let altitude_m = position.altitude_m + climb_m;
    if !distance.is_finite() || !radius.is_finite() || radius <= 0.0 || !altitude_m.is_finite() {
        return None;
    }
    if distance == 0.0 {
        return Some(GeoPose {
            altitude_m,
            ..position
        });
    }
    let angle = distance / radius;
    if !angle.is_finite() {
        return None;
    }
    let latitude = position.latitude_deg.to_radians();
    let longitude = position.longitude_deg.to_radians();
    let north = [
        -latitude.sin() * longitude.cos(),
        -latitude.sin() * longitude.sin(),
        latitude.cos(),
    ];
    let east = [-longitude.sin(), longitude.cos(), 0.0];
    let start = unit_vector(position);
    let direction = std::array::from_fn::<_, 3, _>(|index| {
        north[index] * (north_m / distance) + east[index] * (east_m / distance)
    });
    let destination =
        std::array::from_fn(|index| start[index] * angle.cos() + direction[index] * angle.sin());
    Some(position_from_vector(destination, altitude_m))
}

/// Interpolate the shortest spherical arc and altitude between authored waypoints.
/// Exactly antipodal endpoints have no unique shortest route; choose the initial
/// northward great circle deterministically. Explicit intermediate points remove that ambiguity.
pub fn interpolate_position(a: GeoPose, b: GeoPose, fraction: f64) -> Option<GeoPose> {
    if !crate::geo_pose_is_finite(a)
        || !crate::geo_pose_is_finite(b)
        || !fraction.is_finite()
        || !(0.0..=1.0).contains(&fraction)
    {
        return None;
    }
    if fraction == 0.0 {
        return Some(a);
    }
    if fraction == 1.0 {
        return Some(b);
    }
    let start = unit_vector(a);
    let end = unit_vector(b);
    let angle = central_angle_rad(a, b);
    // Weighted endpoints avoid overflow when finite altitudes have opposite signs.
    let altitude_m = a.altitude_m * (1.0 - fraction) + b.altitude_m * fraction;
    if !altitude_m.is_finite() {
        return None;
    }
    let destination = if angle < 1.0e-12 {
        std::array::from_fn(|index| start[index] * (1.0 - fraction) + end[index] * fraction)
    } else {
        let dot = start
            .iter()
            .zip(end)
            .map(|(x, y)| x * y)
            .sum::<f64>()
            .clamp(-1.0, 1.0);
        let mut tangent = std::array::from_fn::<_, 3, _>(|index| end[index] - dot * start[index]);
        let length = tangent[0].hypot(tangent[1]).hypot(tangent[2]);
        if length < 1.0e-12 {
            let latitude = a.latitude_deg.to_radians();
            let longitude = a.longitude_deg.to_radians();
            tangent = [
                -latitude.sin() * longitude.cos(),
                -latitude.sin() * longitude.sin(),
                latitude.cos(),
            ];
        } else {
            for component in &mut tangent {
                *component /= length;
            }
        }
        let partial = angle * fraction;
        std::array::from_fn(|index| start[index] * partial.cos() + tangent[index] * partial.sin())
    };
    Some(position_from_vector(destination, altitude_m))
}

fn unit_vector(position: GeoPose) -> [f64; 3] {
    let latitude = position.latitude_deg.to_radians();
    let longitude = position.longitude_deg.to_radians();
    [
        latitude.cos() * longitude.cos(),
        latitude.cos() * longitude.sin(),
        latitude.sin(),
    ]
}

fn position_from_vector(vector: [f64; 3], altitude_m: f64) -> GeoPose {
    GeoPose {
        latitude_deg: vector[2].atan2(vector[0].hypot(vector[1])).to_degrees(),
        longitude_deg: vector[1].atan2(vector[0]).to_degrees(),
        altitude_m,
    }
}

fn central_angle_rad(a: GeoPose, b: GeoPose) -> f64 {
    if !crate::geo_pose_is_finite(a) || !crate::geo_pose_is_finite(b) {
        return f64::NAN;
    }
    let d_lat = (b.latitude_deg - a.latitude_deg).to_radians();
    let d_lon = (b.longitude_deg - a.longitude_deg).to_radians();
    let haversine = (d_lat / 2.0).sin().powi(2)
        + a.latitude_deg.to_radians().cos()
            * b.latitude_deg.to_radians().cos()
            * (d_lon / 2.0).sin().powi(2);
    2.0 * haversine.clamp(0.0, 1.0).sqrt().asin()
}

fn horizon_angle_rad(altitude_m: f64) -> f64 {
    // atan2 gives the same exact angle as acos(R / (R + h)), without losing small heights.
    (altitude_m.sqrt() * (2.0 * EARTH_RADIUS_M + altitude_m).sqrt()).atan2(EARTH_RADIUS_M)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(latitude_deg: f64, longitude_deg: f64, altitude_m: f64) -> GeoPose {
        GeoPose {
            latitude_deg,
            longitude_deg,
            altitude_m,
        }
    }

    #[test]
    fn measures_vertical_separation_instead_of_zero_surface_distance() {
        let surface = pose(0.0, 0.0, 0.0);
        let overhead = pose(0.0, 0.0, 10_000.0);
        assert_eq!(surface_distance_m(surface, overhead), 0.0);
        assert_eq!(slant_distance_m(surface, overhead), 10_000.0);
        assert!(has_geometric_line_of_sight(surface, overhead));
    }

    #[test]
    fn rising_aircraft_comes_into_view_over_the_horizon() {
        let observer = pose(0.0, 0.0, 20.0);
        let low_target = pose(0.0, 1.0, 20.0);
        assert!(!has_geometric_line_of_sight(observer, low_target));
        assert!(has_geometric_line_of_sight(
            observer,
            pose(0.0, 1.0, 1_500.0)
        ));
    }

    #[test]
    fn endpoint_horizons_include_target_height_and_tangency() {
        let height = 1_000.0;
        let horizon = (EARTH_RADIUS_M / (EARTH_RADIUS_M + height)).acos();
        let observer = pose(0.0, 0.0, height);
        let tangent = pose(0.0, (2.0 * horizon).to_degrees(), height);
        assert!(has_geometric_line_of_sight(observer, tangent));
        assert!(has_geometric_line_of_sight(tangent, observer));
        let beyond = pose(0.0, (2.0 * horizon + 1.0e-6).to_degrees(), height);
        assert!(!has_geometric_line_of_sight(observer, beyond));
        let surface_target = pose(0.0, 1.5 * horizon.to_degrees(), 0.0);
        assert!(!has_geometric_line_of_sight(observer, surface_target));
    }

    #[test]
    fn geography_handles_antimeridian_poles_and_antipodes() {
        let a = pose(0.0, 179.9, 1_000.0);
        let b = pose(0.0, -179.9, 1_000.0);
        let expected = EARTH_RADIUS_M * 0.2_f64.to_radians();
        assert!((surface_distance_m(a, b) - expected).abs() < 1.0e-6);
        assert!(has_geometric_line_of_sight(a, b));
        let pole_a = pose(90.0, -180.0, 0.0);
        let pole_b = pose(90.0, 180.0, 0.0);
        assert!(surface_distance_m(pole_a, pole_b) < 1.0e-6);
        assert!(has_geometric_line_of_sight(pole_a, pole_b));
        let antipode = pose(-12.345, -101.088, 0.0);
        let original = pose(12.345, 78.912, 0.0);
        assert!(surface_distance_m(original, antipode).is_finite());
        assert!(
            (surface_distance_m(original, antipode) - std::f64::consts::PI * EARTH_RADIUS_M).abs()
                < 1.0
        );
        assert!(!has_geometric_line_of_sight(original, antipode));
    }

    #[test]
    fn invalid_or_submerged_positions_do_not_produce_geometric_visibility() {
        let surface = pose(0.0, 0.0, 0.0);
        for invalid in [
            pose(f64::NAN, 0.0, 0.0),
            pose(91.0, 0.0, 1.0),
            pose(0.0, 181.0, 1.0),
            pose(0.0, 0.0, f64::INFINITY),
        ] {
            assert!(!has_geometric_line_of_sight(surface, invalid));
            assert!(surface_distance_m(surface, invalid).is_nan());
            assert!(slant_distance_m(surface, invalid).is_nan());
        }
        assert!(!has_geometric_line_of_sight(surface, pose(0.0, 0.0, -1.0)));
    }
    #[test]
    fn spherical_motion_crosses_the_antimeridian_at_the_commanded_distance() {
        let start = pose(0.0, 179.999, 0.0);
        let end = advance_position(start, 0.0, 500.0, 0.0).unwrap();
        assert!(end.longitude_deg < -179.99);
        assert!((surface_distance_m(start, end) - 500.0).abs() < 1.0e-6);
        assert!(end.latitude_deg.abs() < 1.0e-10);
        let westbound = advance_position(pose(0.0, -179.999, 0.0), 0.0, -500.0, 0.0).unwrap();
        assert!(westbound.longitude_deg > 179.99);
    }

    #[test]
    fn spherical_motion_crosses_both_poles_and_starts_at_exact_poles() {
        for (latitude, north) in [(89.999, 500.0), (-89.999, -500.0)] {
            let start = pose(latitude, 20.0, 0.0);
            let end = advance_position(start, north, 0.0, 0.0).unwrap();
            assert!(crate::geo_pose_is_finite(end));
            assert!((end.longitude_deg + 160.0).abs() < 1.0e-7);
            assert!(end.latitude_deg.abs() < start.latitude_deg.abs());
            assert!((surface_distance_m(start, end) - 500.0).abs() < 1.0e-6);
        }
        for latitude in [-90.0, 90.0] {
            let start = pose(latitude, 0.0, 0.0);
            for (north, east) in [(500.0, 0.0), (0.0, 500.0), (-500.0, 0.0)] {
                let end = advance_position(start, north, east, 0.0).unwrap();
                assert!(crate::geo_pose_is_finite(end));
                assert!((surface_distance_m(start, end) - 500.0).abs() < 1.0e-6);
            }
        }
    }

    #[test]
    fn movement_preserves_stationary_coordinates_and_accounts_for_starting_altitude() {
        let start = pose(30.0, 40.0, 8_000.0);
        let stopped = advance_position(start, 0.0, 0.0, 20.0).unwrap();
        assert_eq!(stopped.latitude_deg, start.latitude_deg);
        assert_eq!(stopped.longitude_deg, start.longitude_deg);
        assert_eq!(stopped.altitude_m, 8_020.0);
        let moved = advance_position(start, 300.0, 400.0, 20.0).unwrap();
        let ground_distance = 500.0 * EARTH_RADIUS_M / (EARTH_RADIUS_M + start.altitude_m);
        assert!((surface_distance_m(start, moved) - ground_distance).abs() < 1.0e-6);
        assert_eq!(moved.altitude_m, 8_020.0);
    }

    #[test]
    fn waypoint_arc_crosses_the_dateline_and_passes_over_a_pole() {
        let start = pose(15.0, 179.0, 1_000.0);
        let end = pose(15.0, -179.0, 5_000.0);
        let midpoint = interpolate_position(start, end, 0.5).unwrap();
        assert!((midpoint.longitude_deg.abs() - 180.0).abs() < 1.0e-9);
        assert_eq!(midpoint.altitude_m, 3_000.0);
        assert!(
            (surface_distance_m(start, midpoint) - surface_distance_m(start, end) / 2.0).abs()
                < 1.0e-5
        );
        let north =
            interpolate_position(pose(45.0, 0.0, 0.0), pose(45.0, 180.0, 0.0), 0.5).unwrap();
        assert!((north.latitude_deg - 90.0).abs() < 1.0e-9);
    }

    #[test]
    fn interpolation_handles_coincident_and_antipodal_waypoints_deterministically() {
        let start = pose(0.0, 0.0, 0.0);
        let end = pose(0.0, 180.0, 1_000.0);
        let midpoint = interpolate_position(start, end, 0.5).unwrap();
        assert!((midpoint.latitude_deg - 90.0).abs() < 1.0e-9);
        assert_eq!(midpoint.altitude_m, 500.0);
        assert_eq!(
            interpolate_position(start, end, 0.0).unwrap().longitude_deg,
            0.0
        );
        assert_eq!(
            interpolate_position(start, end, 1.0).unwrap().longitude_deg,
            180.0
        );
        let stationary =
            interpolate_position(pose(45.0, 90.0, 0.0), pose(45.0, 90.0, 1_000.0), 0.5).unwrap();
        assert!((stationary.latitude_deg - 45.0).abs() < 1.0e-10);
        assert!((stationary.longitude_deg - 90.0).abs() < 1.0e-10);
        assert_eq!(stationary.altitude_m, 500.0);
    }

    #[test]
    fn navigation_rejects_invalid_displacements_positions_and_fractions() {
        let start = pose(0.0, 0.0, 0.0);
        for (north, east, climb) in [
            (f64::NAN, 0.0, 0.0),
            (0.0, f64::INFINITY, 0.0),
            (0.0, 0.0, f64::INFINITY),
            (f64::MAX, f64::MAX, 0.0),
        ] {
            assert!(advance_position(start, north, east, climb).is_none());
        }
        assert!(advance_position(pose(91.0, 0.0, 0.0), 1.0, 0.0, 0.0).is_none());
        let tiny_radius = pose(0.0, 0.0, -EARTH_RADIUS_M + 1.0e-6);
        assert!(advance_position(tiny_radius, f64::MAX, 0.0, 0.0).is_none());
        let extreme_altitude =
            interpolate_position(pose(0.0, 0.0, -f64::MAX), pose(0.0, 0.0, f64::MAX), 0.5).unwrap();
        assert_eq!(extreme_altitude.altitude_m, 0.0);
        for fraction in [-0.1, 1.1, f64::NAN] {
            assert!(interpolate_position(start, start, fraction).is_none());
        }
    }
}
