//! Shared, deterministic airspace geometry. MSL heights use EGM96; horizontal edges
//! are shortest great-circle arcs on a mean-radius Earth, including the date line.
use geo::{Area, BooleanOps, Coord, Intersects, Line, LineString, Point, Polygon};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uom::si::{
    f64::Length,
    length::{foot, meter},
};

pub mod geoid;
const R: f64 = 6_371_008.8;
pub const MAX_VERTICES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct MslAltitude(pub Length);
impl MslAltitude {
    pub fn from_meters(value: f64) -> Self {
        Self(Length::new::<meter>(value))
    }
    pub fn from_feet(value: f64) -> Self {
        Self(Length::new::<foot>(value))
    }
    pub fn meters(self) -> f64 {
        self.0.get::<meter>()
    }
    pub fn feet(self) -> f64 {
        self.0.get::<foot>()
    }
}
impl Serialize for MslAltitude {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.meters())
    }
}
impl<'de> Deserialize<'de> for MslAltitude {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_meters(f64::deserialize(d)?))
    }
}
// Length quantities on the wire are explicitly in metres, never datum-bearing heights.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Distance(pub Length);
impl Distance {
    pub fn meters(self) -> f64 {
        self.0.get::<meter>()
    }
    pub fn new(value: f64) -> Self {
        Self(Length::new::<meter>(value))
    }
}
impl Serialize for Distance {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.meters())
    }
}
impl<'de> Deserialize<'de> for Distance {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::new(f64::deserialize(d)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LatLon {
    pub latitude_deg: f64,
    pub longitude_deg: f64,
}
impl LatLon {
    pub fn valid(self) -> bool {
        self.latitude_deg.is_finite()
            && self.longitude_deg.is_finite()
            && self.latitude_deg.abs() <= 90.0
            && self.longitude_deg.abs() <= 180.0
    }
    fn vector(self) -> [f64; 3] {
        let (s, c) = self.latitude_deg.to_radians().sin_cos();
        let (sl, cl) = self.longitude_deg.to_radians().sin_cos();
        [c * cl, c * sl, s]
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: [f64; 3]) -> [f64; 3] {
    let n = dot(a, a).sqrt();
    a.map(|x| x / n)
}
fn location(a: [f64; 3]) -> LatLon {
    let a = norm(a);
    LatLon {
        latitude_deg: a[2].asin().to_degrees(),
        longitude_deg: a[1].atan2(a[0]).to_degrees(),
    }
}
pub fn distance(a: LatLon, b: LatLon) -> f64 {
    dot(a.vector(), b.vector()).clamp(-1.0, 1.0).acos() * R
}
pub fn interpolate(a: LatLon, b: LatLon, t: f64) -> LatLon {
    let a = a.vector();
    let b = b.vector();
    let angle = dot(a, b).clamp(-1.0, 1.0).acos();
    if angle < 1e-10 {
        return location(a);
    }
    let sa = ((1.0 - t) * angle).sin() / angle.sin();
    let sb = (t * angle).sin() / angle.sin();
    location(std::array::from_fn(|i| a[i] * sa + b[i] * sb))
}
pub fn destination(p: LatLon, bearing: f64, meters: f64) -> LatLon {
    let a = meters / R;
    let lat = p.latitude_deg.to_radians();
    let lon = p.longitude_deg.to_radians();
    let next = (lat.sin() * a.cos() + lat.cos() * a.sin() * bearing.cos()).asin();
    LatLon {
        latitude_deg: next.to_degrees(),
        longitude_deg: ((lon
            + (bearing.sin() * a.sin() * lat.cos()).atan2(a.cos() - lat.sin() * next.sin()))
        .to_degrees()
            + 180.0)
            .rem_euclid(360.0)
            - 180.0,
    }
}

// A gnomonic projection turns great-circle edges into straight lines. All
// geometry operands share a projection; no longitude discontinuity is involved.
struct Frame {
    center: [f64; 3],
    east: [f64; 3],
    north: [f64; 3],
}
impl Frame {
    fn new(points: &[LatLon]) -> Result<Self, String> {
        let center = norm(points.iter().fold([0.0; 3], |mut s, p| {
            for (i, v) in p.vector().iter().enumerate() {
                s[i] += v
            }
            s
        }));
        if !center[0].is_finite() {
            return Err("geometry must fit within a hemisphere".into());
        }
        let east = norm(cross([0.0, 0.0, 1.0], center));
        let north = cross(center, east);
        Ok(Self {
            center,
            east,
            north,
        })
    }
    fn project(&self, p: LatLon) -> Result<Coord, String> {
        let p = p.vector();
        let d = dot(p, self.center);
        if d <= 1e-6 {
            return Err("geometry must fit within a hemisphere".into());
        }
        Ok(Coord {
            x: R * dot(p, self.east) / d,
            y: R * dot(p, self.north) / d,
        })
    }
    fn unproject(&self, p: Coord) -> LatLon {
        location(std::array::from_fn(|i| {
            self.center[i] + p.x / R * self.east[i] + p.y / R * self.north[i]
        }))
    }
    fn polygon(&self, points: &[LatLon]) -> Result<Polygon, String> {
        let mut ring = points
            .iter()
            .map(|p| self.project(*p))
            .collect::<Result<Vec<_>, _>>()?;
        if ring.first() != ring.last() {
            ring.push(ring[0]);
        }
        Ok(Polygon::new(LineString::new(ring), vec![]))
    }
}
pub fn normalize_ring(points: &mut Vec<LatLon>) {
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
}
pub fn validate_ring(points: &[LatLon]) -> Result<(), String> {
    let mut points = points.to_vec();
    normalize_ring(&mut points);
    if points.len() < 3 || points.len() > MAX_VERTICES || points.iter().any(|p| !p.valid()) {
        return Err("polygon needs 3–4096 valid latitude/longitude vertices".into());
    }
    let f = Frame::new(&points)?;
    let p = f.polygon(&points)?;
    let lines: Vec<_> = p.exterior().lines().collect();
    for (i, a) in lines.iter().enumerate() {
        if a.start == a.end {
            return Err("duplicate adjacent polygon vertex".into());
        }
        for (j, b) in lines.iter().enumerate().skip(i + 1) {
            if j == i + 1 || (i == 0 && j == lines.len() - 1) {
                continue;
            }
            if a.intersects(b) {
                return Err("polygon intersects or touches itself".into());
            }
        }
    }
    if p.unsigned_area() < 1.0 {
        return Err("polygon area must exceed one square metre".into());
    }
    for pole in [
        LatLon {
            latitude_deg: 90.0,
            longitude_deg: 0.0,
        },
        LatLon {
            latitude_deg: -90.0,
            longitude_deg: 0.0,
        },
    ] {
        if f.project(pole).is_ok_and(|q| p.intersects(&Point(q))) {
            return Err("pole-enclosing polygons are not supported".into());
        }
    }
    Ok(())
}
pub fn contains(points: &[LatLon], point: LatLon) -> bool {
    Frame::new(points)
        .ok()
        .and_then(|f| Some((f.polygon(points).ok()?, f.project(point).ok()?)))
        .is_some_and(|(p, q)| p.intersects(&Point(q)))
}
pub fn polygons_overlap(a: &[LatLon], b: &[LatLon]) -> bool {
    let all: Vec<_> = a.iter().chain(b).copied().collect();
    let Ok(f) = Frame::new(&all) else {
        return false;
    };
    match (f.polygon(a), f.polygon(b)) {
        (Ok(a), Ok(b)) => a.intersection(&b).unsigned_area() > 0.01,
        _ => false,
    }
}
pub fn segment_intersects(points: &[LatLon], a: LatLon, b: LatLon) -> bool {
    let Ok(f) = Frame::new(points) else {
        return false;
    };
    match (f.polygon(points), f.project(a), f.project(b)) {
        (Ok(p), Ok(a), Ok(b)) => p.intersects(&Line::new(a, b)),
        _ => contains(points, a) || contains(points, b),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceGeometry {
    Polygon {
        vertices: Vec<LatLon>,
    },
    Circle {
        center: LatLon,
        radius_m: Distance,
    },
    Corridor {
        centerline: Vec<LatLon>,
        width_m: Distance,
    },
}
impl SourceGeometry {
    pub fn polygon(&self) -> Result<Vec<LatLon>, String> {
        let mut points = match self {
            Self::Polygon { vertices } => vertices.clone(),
            Self::Circle { center, radius_m } => circle(*center, radius_m.meters())?,
            Self::Corridor {
                centerline,
                width_m,
            } => {
                if centerline.len() < 2
                    || centerline.len() > 128
                    || centerline.iter().any(|p| !p.valid())
                {
                    return Err("corridor needs 2–128 valid centerline points".into());
                }
                let radius = width_m.meters() / 2.0;
                let frame = Frame::new(centerline)?;
                let mut union = geo::MultiPolygon(vec![]);
                for pair in centerline.windows(2) {
                    let a = pair[0];
                    let b = pair[1];
                    let length = distance(a, b);
                    if !(1.0..=1_000_000.0).contains(&length) {
                        return Err("corridor segments must be 1 m–1000 km".into());
                    }
                    // Densified disks and side strips form a round-ended geodesic corridor.
                    let n = (length / 5000.0).ceil() as usize;
                    for i in 0..=n {
                        let c = interpolate(a, b, i as f64 / n as f64);
                        let ring = circle(c, radius)?;
                        union = union.union(&geo::MultiPolygon(vec![frame.polygon(&ring)?]));
                        if i < n {
                            let d = interpolate(a, b, (i + 1) as f64 / n as f64);
                            let bearing = bearing(c, d);
                            let ring = [
                                destination(c, bearing - std::f64::consts::FRAC_PI_2, radius),
                                destination(d, bearing - std::f64::consts::FRAC_PI_2, radius),
                                destination(d, bearing + std::f64::consts::FRAC_PI_2, radius),
                                destination(c, bearing + std::f64::consts::FRAC_PI_2, radius),
                            ];
                            union = union.union(&geo::MultiPolygon(vec![frame.polygon(&ring)?]));
                        }
                    }
                }
                if union.0.len() != 1 || !union.0[0].interiors().is_empty() {
                    return Err("corridor must form one region without holes".into());
                }
                union.0[0]
                    .exterior()
                    .0
                    .iter()
                    .map(|p| frame.unproject(*p))
                    .collect()
            }
        };
        normalize_ring(&mut points);
        validate_ring(&points)?;
        Ok(points)
    }
}
fn bearing(a: LatLon, b: LatLon) -> f64 {
    let la = a.latitude_deg.to_radians();
    let lb = b.latitude_deg.to_radians();
    let dl = (b.longitude_deg - a.longitude_deg).to_radians();
    (dl.sin() * lb.cos()).atan2(la.cos() * lb.sin() - la.sin() * lb.cos() * dl.cos())
}
fn circle(center: LatLon, r: f64) -> Result<Vec<LatLon>, String> {
    if !center.valid() || !r.is_finite() || !(1.0..=500_000.0).contains(&r) {
        return Err("radius must be 1 m–500 km".into());
    }
    let n = (std::f64::consts::PI / (1.0 - 10.0_f64.min(r / 2.0) / r).acos())
        .ceil()
        .max(16.0) as usize;
    Ok((0..n)
        .map(|i| destination(center, i as f64 / n as f64 * std::f64::consts::TAU, r))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(lat: f64, lon: f64) -> LatLon {
        LatLon {
            latitude_deg: lat,
            longitude_deg: lon,
        }
    }
    #[test]
    fn units_are_explicit() {
        assert!((MslAltitude::from_feet(1000.0).meters() - 304.8).abs() < 1e-9);
        assert_eq!(
            serde_json::to_string(&MslAltitude::from_meters(-10.0)).unwrap(),
            "-10.0"
        );
    }
    #[test]
    fn date_line_and_invalid_rings() {
        let ring = vec![p(10., 179.), p(10., -179.), p(12., -179.), p(12., 179.)];
        validate_ring(&ring).unwrap();
        assert!(contains(&ring, p(11., 180.)));
        assert!(!contains(&ring, p(11., 0.)));
        assert!(validate_ring(&[p(0., 0.), p(1., 1.), p(0., 1.), p(1., 0.)]).is_err());
    }
    #[test]
    fn overlap_is_not_a_bounding_box_test() {
        let a = vec![p(0., 0.), p(0., 2.), p(2., 0.)];
        let b = vec![p(2., 2.), p(2., 1.2), p(1.2, 2.)];
        assert!(!polygons_overlap(&a, &b));
        assert!(polygons_overlap(&a, &a));
    }
    #[test]
    fn shapes_normalize() {
        for g in [
            SourceGeometry::Circle {
                center: p(10., 179.99),
                radius_m: Distance::new(5000.),
            },
            SourceGeometry::Corridor {
                centerline: vec![p(10., 179.9), p(10., -179.9)],
                width_m: Distance::new(2000.),
            },
        ] {
            validate_ring(&g.polygon().unwrap()).unwrap();
        }
    }
}
