//! Pinned NGA EGM96 grid; all consumers use the same bilinear interpolation.
use sha2::{Digest, Sha256};
pub const GRID: &[u8] = include_bytes!("../../../data/geo/egm96-15.f32");
pub const SHA256: &str = "5c720c5f1b5a247414c59d9d22f79183ab4431c699a18a6e904edc193bf63ed4";
pub fn validate() -> Result<(), String> {
    if GRID.len() != 721 * 1441 * 4 || format!("{:x}", Sha256::digest(GRID)) != SHA256 {
        Err("EGM96 grid checksum mismatch".into())
    } else {
        Ok(())
    }
}
pub fn undulation(latitude: f64, longitude: f64) -> f64 {
    let row = ((90.0 - latitude.clamp(-90.0, 90.0)) * 4.0).min(720.0);
    let col = longitude.rem_euclid(360.0) * 4.0;
    let y = (row.floor() as usize).min(719);
    let x = (col.floor() as usize).min(1439);
    let fy = row - y as f64;
    let fx = col - x as f64;
    let value = |r: usize, c: usize| {
        let i = (r * 1441 + c) * 4;
        f32::from_le_bytes(GRID[i..i + 4].try_into().unwrap()) as f64
    };
    (1.0 - fy) * ((1.0 - fx) * value(y, x) + fx * value(y, x + 1))
        + fy * ((1.0 - fx) * value(y + 1, x) + fx * value(y + 1, x + 1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_points_and_seam() {
        validate().unwrap();
        for (lat, lon, expected) in [
            (38.628155, 269.779155, -31.628),
            (-14.621217, 305.021114, -2.969),
            (46.874319, 102.448729, -43.575),
            (-23.617446, 133.874712, 15.871),
        ] {
            assert!((undulation(lat, lon) - expected).abs() < 0.2);
        }
        assert_eq!(undulation(20., -180.), undulation(20., 180.));
    }
}
