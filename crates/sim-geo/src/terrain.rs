//! Offline, checksummed SRTM HGT terrain. Heights are metres EGM96 MSL.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TilePin {
    pub file: String,
    pub latitude: i32,
    pub longitude: i32,
    pub samples: usize,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub source: String,
    pub vertical_datum: String,
    pub tiles: Vec<TilePin>,
}
impl Manifest {
    pub fn digest(&self) -> String {
        format!("{:x}", Sha256::digest(serde_json_bytes(self)))
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.vertical_datum != "EGM96"
            || self.source.trim().is_empty()
            || self.tiles.is_empty()
        {
            return Err("invalid terrain manifest".into());
        }
        let mut keys = std::collections::BTreeSet::new();
        for tile in &self.tiles {
            if ![1201, 3601].contains(&tile.samples)
                || !(-90..90).contains(&tile.latitude)
                || !(-180..180).contains(&tile.longitude)
                || tile.sha256.len() != 64
                || !tile.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || Path::new(&tile.file).components().count() != 1
                || !tile.file.ends_with(".hgt")
                || !keys.insert((tile.latitude, tile.longitude))
            {
                return Err("invalid or duplicate terrain tile".into());
            }
        }
        Ok(())
    }
}
fn serde_json_bytes(manifest: &Manifest) -> Vec<u8> {
    serde_json::to_vec(manifest).expect("finite terrain manifest")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub latitude: i32,
    pub longitude: i32,
    pub samples: usize,
    pub heights: Vec<i16>,
}
impl Grid {
    pub fn terrain(&self) -> Result<Arc<Terrain>, String> {
        if !(2..=1201).contains(&self.samples)
            || self.heights.len() != self.samples * self.samples
            || !(-90..90).contains(&self.latitude)
            || !(-180..180).contains(&self.longitude)
        {
            return Err("invalid synthetic terrain grid".into());
        }
        let bytes: Vec<_> = self.heights.iter().flat_map(|v| v.to_be_bytes()).collect();
        let pin = TilePin {
            file: "synthetic.hgt".into(),
            latitude: self.latitude,
            longitude: self.longitude,
            samples: self.samples,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        };
        Ok(Arc::new(Terrain {
            manifest: Manifest {
                version: 1,
                source: "Fictional ridge fixture".into(),
                vertical_datum: "EGM96".into(),
                tiles: vec![pin.clone()],
            },
            tiles: BTreeMap::from([(
                (self.latitude, self.longitude),
                Tile {
                    pin,
                    heights: self.heights.clone(),
                },
            )]),
        }))
    }
}

#[derive(Debug, Clone)]
struct Tile {
    pin: TilePin,
    heights: Vec<i16>,
}
#[derive(Debug, Clone)]
pub struct Terrain {
    pub manifest: Manifest,
    tiles: BTreeMap<(i32, i32), Tile>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Clear,
    Blocked,
    Unknown,
}
impl Terrain {
    pub fn load(root: &Path, expected: Option<&Manifest>) -> Result<Arc<Self>, String> {
        let bytes = std::fs::read(root.join("manifest.json"))
            .map_err(|e| format!("Terrain setup required at {}: {e}", root.display()))?;
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        manifest.validate()?;
        if expected.is_some_and(|pin| pin != &manifest) {
            return Err("installed terrain differs from the saved game pin".into());
        }
        let mut tiles = BTreeMap::new();
        for pin in &manifest.tiles {
            let bytes = std::fs::read(root.join(&pin.file)).map_err(|e| e.to_string())?;
            if bytes.len() != pin.samples * pin.samples * 2
                || format!("{:x}", Sha256::digest(&bytes)) != pin.sha256
            {
                return Err(format!(
                    "terrain tile checksum or dimensions mismatch: {}",
                    pin.file
                ));
            }
            tiles.insert(
                (pin.latitude, pin.longitude),
                Tile {
                    pin: pin.clone(),
                    heights: bytes
                        .chunks_exact(2)
                        .map(|b| i16::from_be_bytes([b[0], b[1]]))
                        .collect(),
                },
            );
        }
        Ok(Arc::new(Self { manifest, tiles }))
    }
    fn cell(&self, lat: f64, lon: f64) -> Option<([f64; 4], f64, f64)> {
        if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) {
            return None;
        }
        let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        let tile = self
            .tiles
            .get(&(lat.floor() as i32, lon.floor() as i32))
            .or_else(|| {
                // Shared north/east edge may use the adjacent installed tile.
                self.tiles.values().find(|t| {
                    lat >= t.pin.latitude as f64
                        && lat <= t.pin.latitude as f64 + 1.0
                        && lon >= t.pin.longitude as f64
                        && lon <= t.pin.longitude as f64 + 1.0
                })
            })?;
        let n = tile.pin.samples;
        let row = (tile.pin.latitude as f64 + 1.0 - lat) * (n - 1) as f64;
        let col = (lon - tile.pin.longitude as f64) * (n - 1) as f64;
        let y = (row.floor() as usize).min(n - 2);
        let x = (col.floor() as usize).min(n - 2);
        let values = [
            tile.heights[y * n + x],
            tile.heights[y * n + x + 1],
            tile.heights[(y + 1) * n + x],
            tile.heights[(y + 1) * n + x + 1],
        ];
        if values.contains(&i16::MIN) {
            return None;
        }
        Some((values.map(f64::from), row - y as f64, col - x as f64))
    }
    pub fn height(&self, lat: f64, lon: f64) -> Option<f64> {
        let (h, fy, fx) = self.cell(lat, lon)?;
        Some((1.0 - fy) * ((1.0 - fx) * h[0] + fx * h[1]) + fy * ((1.0 - fx) * h[2] + fx * h[3]))
    }
    /// Conservative cell envelope prevents a sub-sample ridge being missed.
    /// Segment positions use spherical chord geometry, not linear height over an arc.
    pub fn visibility(&self, a: [f64; 3], b: [f64; 3]) -> Visibility {
        const R: f64 = 6_371_000.0;
        let vector = |p: [f64; 3]| {
            let lat = p[0].to_radians();
            let lon = p[1].to_radians();
            let r = R + p[2];
            [
                r * lat.cos() * lon.cos(),
                r * lat.cos() * lon.sin(),
                r * lat.sin(),
            ]
        };
        for p in [a, b] {
            match self.height(p[0], p[1]) {
                Some(h) if p[2] + 0.01 >= h => {}
                Some(_) => return Visibility::Blocked,
                None => return Visibility::Unknown,
            }
        }
        let av = vector(a);
        let bv = vector(b);
        let distance = (0..3).map(|i| (bv[i] - av[i]).powi(2)).sum::<f64>().sqrt();
        let steps = (distance / 15.0).ceil().max(1.0) as usize;
        if steps > 100_000 {
            return Visibility::Unknown;
        }
        let mut unknown = false;
        for i in 1..steps {
            let f = i as f64 / steps as f64;
            let p: [f64; 3] = std::array::from_fn(|j| av[j] * (1.0 - f) + bv[j] * f);
            let r = p.iter().map(|x| x * x).sum::<f64>().sqrt();
            let lat = (p[2] / r).asin().to_degrees();
            let lon = p[1].atan2(p[0]).to_degrees();
            match self.cell(lat, lon) {
                Some((h, _, _))
                    if r - R + 0.01 < h.into_iter().fold(f64::NEG_INFINITY, f64::max) =>
                {
                    return Visibility::Blocked
                }
                Some(_) => {}
                None => unknown = true,
            }
        }
        if unknown {
            Visibility::Unknown
        } else {
            Visibility::Clear
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Terrain {
        let pin = TilePin {
            file: "N00E000.hgt".into(),
            latitude: 0,
            longitude: 0,
            samples: 1201,
            sha256: "0".repeat(64),
        };
        let mut heights = vec![0; 1201 * 1201];
        for y in 0..1201 {
            heights[y * 1201 + 600] = 500;
        }
        Terrain {
            manifest: Manifest {
                version: 1,
                source: "synthetic ridge".into(),
                vertical_datum: "EGM96".into(),
                tiles: vec![pin.clone()],
            },
            tiles: BTreeMap::from([((0, 0), Tile { pin, heights })]),
        }
    }
    #[test]
    fn ridge_blocks_low_ray_and_climb_clears_it() {
        let terrain = fixture();
        assert_eq!(
            terrain.visibility([0.5, 0.49, 100.0], [0.5, 0.51, 100.0]),
            Visibility::Blocked
        );
        assert_eq!(
            terrain.visibility([0.5, 0.49, 1000.0], [0.5, 0.51, 1000.0]),
            Visibility::Clear
        );
        assert_eq!(
            terrain.visibility([0.5, -0.1, 1000.0], [0.5, 0.51, 1000.0]),
            Visibility::Unknown
        );
    }
    #[test]
    fn voids_and_invalid_manifest_are_not_flat_ground() {
        let mut terrain = fixture();
        terrain.tiles.get_mut(&(0, 0)).unwrap().heights[600 * 1201 + 600] = i16::MIN;
        assert_eq!(terrain.height(0.5, 0.5), None);
        let mut manifest = terrain.manifest.clone();
        manifest.tiles[0].file = "../escape.hgt".into();
        assert!(manifest.validate().is_err());
    }
}
