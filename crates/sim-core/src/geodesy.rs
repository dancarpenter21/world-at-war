//! Spherical geometry for the prototype's geometric sensor model.
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
}
