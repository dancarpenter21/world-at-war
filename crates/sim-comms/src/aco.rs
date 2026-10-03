//! Text ACO importer. Supported dialects are fixture-defined, never silently guessed.
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sim_geo::{Distance, LatLon, MslAltitude, SourceGeometry};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Resolution {
    #[serde(default)]
    pub exclude: bool,
    pub floor_m: Option<MslAltitude>,
    pub ceiling_m: Option<MslAltitude>,
    pub controller_role_id: Option<String>,
    pub kind: Option<String>,
    pub geometry: Option<SourceGeometry>,
    pub periods: Option<Vec<Period>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    pub anchor_utc: String,
    pub anchor_tick: u64,
    pub year: i32,
    pub horizon_end_utc: String,
    #[serde(default)]
    pub resolutions: BTreeMap<String, Resolution>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Period {
    pub start_tick: u64,
    pub end_tick: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Create,
    Change,
    Cancel,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub external_id: String,
    pub name: String,
    pub action: Action,
    pub geometry: Option<SourceGeometry>,
    pub polygon: Vec<LatLon>,
    pub floor_m: Option<MslAltitude>,
    pub ceiling_m: Option<MslAltitude>,
    pub agency: Option<String>,
    pub kind: String,
    pub controller_role_id: Option<String>,
    pub periods: Vec<Period>,
    pub raw: String,
    pub line: usize,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
    pub excluded: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preview {
    pub order_id: String,
    pub source_hash: String,
    pub records: Vec<Record>,
    pub issues: Vec<String>,
}

fn value(s: &str) -> &str {
    s.split_once(':').map_or(s, |(_, v)| v).trim()
}
pub fn parse_coordinate(text: &str) -> Result<LatLon, String> {
    let text = value(text).trim().replace(' ', "");
    if let Some((lat, lon)) = text.split_once(',') {
        let p = LatLon {
            latitude_deg: lat.parse().map_err(|_| "invalid latitude")?,
            longitude_deg: lon.parse().map_err(|_| "invalid longitude")?,
        };
        return p
            .valid()
            .then_some(p)
            .ok_or("coordinate out of range".into());
    }
    let i = text
        .find(['N', 'S'])
        .ok_or("expected latitude hemisphere")?;
    let (lat, lon) = text.split_at(i + 1);
    fn angle(s: &str, degree_digits: usize) -> Result<f64, String> {
        if s.len() < degree_digits + 1 {
            return Err("invalid coordinate".into());
        }
        let sign = if s.ends_with(['S', 'W']) {
            -1.0
        } else if s.ends_with(['N', 'E']) {
            1.0
        } else {
            return Err("missing hemisphere".into());
        };
        let digits = &s[..s.len() - 1];
        if !digits.is_ascii() {
            return Err("coordinate must be ASCII".into());
        }
        let deg: f64 = digits[..degree_digits]
            .parse()
            .map_err(|_| "invalid degrees")?;
        let rem = &digits[degree_digits..];
        let (min, sec) = if rem.is_empty() {
            (0.0, 0.0)
        } else if rem.len() >= 4 && !rem[..2].contains('.') {
            (
                rem[..2].parse::<f64>().map_err(|_| "invalid minutes")?,
                rem[2..].parse::<f64>().map_err(|_| "invalid seconds")?,
            )
        } else {
            (rem.parse::<f64>().map_err(|_| "invalid minutes")?, 0.0)
        };
        if min >= 60.0 || sec >= 60.0 {
            return Err("minutes/seconds out of range".into());
        }
        Ok(sign * (deg + min / 60.0 + sec / 3600.0))
    }
    let p = LatLon {
        latitude_deg: angle(lat, 2)?,
        longitude_deg: angle(lon, 3)?,
    };
    p.valid()
        .then_some(p)
        .ok_or("coordinate out of range".into())
}
fn length(s: &str) -> Result<f64, String> {
    let s = value(s).trim();
    for (suffix, mult) in [("NM", 1852.0), ("KM", 1000.0), ("FT", 0.3048), ("M", 1.0)] {
        if let Some(n) = s.strip_suffix(suffix) {
            return n
                .trim()
                .parse::<f64>()
                .map(|n| n * mult)
                .map_err(|_| "invalid dimension".into());
        }
    }
    Err("dimension needs M, FT, KM or NM units".into())
}
fn altitude(s: &str) -> Result<MslAltitude, String> {
    let s = value(s).trim();
    if s.contains("AGL")
        || s.starts_with("FL")
        || ["SFC", "SURFACE", "UNL", "UNLIMITED"].contains(&s)
    {
        return Err(
            "explicit MSL resolution required for AGL, flight level, surface or unlimited bound"
                .into(),
        );
    }
    if let Some(n) = s.strip_suffix("AMSL") {
        return n
            .parse::<f64>()
            .map(MslAltitude::from_feet)
            .map_err(|_| "invalid AMSL feet".into());
    }
    if let Some(n) = s.strip_suffix("MSL") {
        return length(n).map(MslAltitude::from_meters);
    }
    Err("altitude needs AMSL (feet), FTMSL or MMSL units".into())
}
fn dtg(s: &str, year: i32) -> Result<DateTime<Utc>, String> {
    let s = if s.contains('T') && s.contains('-') {
        s
    } else {
        value(s)
    };
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    let z = s.find('Z').ok_or("date needs UTC Z")?;
    let (digits, tail) = s.split_at(z);
    if digits.len() != 6 || !digits.is_ascii() {
        return Err("expected DDHHMMZMON[YYYY]".into());
    }
    let tail = &tail[1..];
    if tail.len() < 3 || !tail.is_ascii() {
        return Err("date needs month".into());
    }
    let month = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ]
    .iter()
    .position(|m| *m == &tail[..3])
    .ok_or("invalid month")?
        + 1;
    let year = if tail.len() > 3 {
        tail[3..].parse().map_err(|_| "invalid year")?
    } else {
        year
    };
    let num = |r: std::ops::Range<usize>| digits[r].parse::<u32>().map_err(|_| "invalid date");
    let date = NaiveDate::from_ymd_opt(year, month as u32, num(0..2)?)
        .and_then(|d| d.and_hms_opt(num(2..4).ok()?, num(4..6).ok()?, 0))
        .ok_or("invalid date")?;
    Ok(Utc.from_utc_datetime(&date))
}
fn periods(fields: &[String], options: &ImportOptions) -> Result<Vec<Period>, String> {
    let anchor = DateTime::parse_from_rfc3339(&options.anchor_utc)
        .map_err(|_| "invalid UTC anchor")?
        .with_timezone(&Utc);
    let horizon = DateTime::parse_from_rfc3339(&options.horizon_end_utc)
        .map_err(|_| "invalid horizon")?
        .with_timezone(&Utc);
    if horizon <= anchor || horizon - anchor > Duration::days(366) {
        return Err("import horizon must be after anchor and within 366 days".into());
    }
    let mode = fields.first().map(|s| value(s.as_str())).unwrap_or("");
    let daily = mode == "DAILY";
    let skip = usize::from(["DAILY", "DISCRETE", "CONTINUOUS"].contains(&mode));
    if fields.len() != skip + 2 {
        return Err("period requires start/end; supported recurrence is DAILY".into());
    }
    let start = dtg(&fields[skip], options.year)?;
    let mut end = dtg(&fields[skip + 1], start.year())?;
    if end < start && end.month() < start.month() {
        end = dtg(&fields[skip + 1], start.year() + 1)?;
    }
    if end <= start {
        return Err("period end must follow start".into());
    }
    let to_tick = |d: DateTime<Utc>| -> Result<u64, String> {
        let seconds = (d - anchor).num_seconds();
        let t = i128::from(options.anchor_tick) + i128::from(seconds);
        u64::try_from(t).map_err(|_| "period precedes simulation tick zero".into())
    };
    let mut result = vec![];
    let mut a = start;
    let mut b = end;
    while a < horizon {
        if b > horizon {
            return Err("period exceeds confirmed import horizon".into());
        }
        result.push(Period {
            start_tick: to_tick(a)?,
            end_tick: to_tick(b)?,
        });
        if !daily {
            break;
        }
        a += Duration::days(1);
        b += Duration::days(1);
        if result.len() > 366 {
            break;
        }
    }
    if result.is_empty() {
        return Err("period is outside import horizon".into());
    }
    Ok(result)
}
struct Builder {
    record: Record,
    coords: Vec<LatLon>,
    shape: String,
    size: Option<f64>,
    period_sets: Vec<Vec<String>>,
    limits: Option<Vec<String>>,
    geometry_errors: Vec<String>,
}
fn finish(mut b: Builder, options: &ImportOptions) -> Record {
    let r = &mut b.record;
    if r.action != Action::Cancel {
        r.geometry =
            match b.shape.as_str() {
                "POLYGON" => Some(SourceGeometry::Polygon { vertices: b.coords }),
                "CIRCLE" => b.coords.first().copied().zip(b.size).map(|(center, r)| {
                    SourceGeometry::Circle {
                        center,
                        radius_m: Distance::new(r),
                    }
                }),
                "CORRIDOR" => b.size.map(|width| SourceGeometry::Corridor {
                    centerline: b.coords,
                    width_m: Distance::new(width),
                }),
                _ => None,
            };
        let res = options.resolutions.get(&r.external_id);
        if let Some(g) = res.and_then(|r| r.geometry.clone()) {
            r.geometry = Some(g);
        } else {
            r.issues.extend(b.geometry_errors);
        }
        match r
            .geometry
            .as_ref()
            .ok_or("missing or unsupported geometry".to_string())
            .and_then(SourceGeometry::polygon)
        {
            Ok(p) => r.polygon = p,
            Err(e) => r.issues.push(e),
        }
        if b.shape != "POLYGON" {
            r.warnings
                .push("Curved boundary is polygonized (target error <=25 m)".into());
        }
        let bounds = b.limits.as_ref().and_then(|f| {
            if f.len() == 2 {
                Some((value(&f[0]), value(&f[1])))
            } else {
                f.first().and_then(|s| value(s).split_once('-'))
            }
        });
        let parsed = bounds.map(|(a, b)| (altitude(a), altitude(b)));
        r.floor_m = res
            .and_then(|v| v.floor_m)
            .or_else(|| parsed.as_ref().and_then(|(a, _)| a.clone().ok()));
        r.ceiling_m = res
            .and_then(|v| v.ceiling_m)
            .or_else(|| parsed.as_ref().and_then(|(_, b)| b.clone().ok()));
        if r.floor_m.is_none() || r.ceiling_m.is_none() {
            r.issues.push("Resolve both vertical limits to MSL".into())
        }
        if let (Some(a), Some(b)) = (r.floor_m, r.ceiling_m) {
            if !a.meters().is_finite() || !b.meters().is_finite() || a >= b {
                r.issues.push("MSL floor must be below ceiling".into())
            }
        }
        if let Some(p) = res.and_then(|v| v.periods.clone()) {
            r.periods = p;
        } else {
            for p in b.period_sets {
                match periods(&p, options) {
                    Ok(p) => r.periods.extend(p),
                    Err(e) => r.issues.push(e),
                }
            }
        }
        r.periods.sort_by_key(|p| p.start_tick);
        r.periods.dedup();
        if r.periods.is_empty()
            || r.periods.iter().any(|p| p.start_tick >= p.end_tick)
            || r.periods
                .windows(2)
                .any(|p| p[0].end_tick > p[1].start_tick)
        {
            r.issues
                .push("Resolve non-overlapping activation periods".into())
        }
        if let Some(res) = res {
            r.controller_role_id = res.controller_role_id.clone();
            if let Some(kind) = &res.kind {
                r.kind = kind.clone();
            }
        }
        if r.controller_role_id.is_none() {
            r.issues
                .push("Map the controlling agency to a campaign role".into());
        }
        if !["sector", "corridor", "restricted", "patrol"].contains(&r.kind.as_str()) {
            r.issues
                .push("Map the airspace purpose to a simulation type".into());
        }
    }
    r.excluded = options
        .resolutions
        .get(&r.external_id)
        .is_some_and(|r| r.exclude);
    b.record
}
pub fn parse(source: &str, options: &ImportOptions) -> Preview {
    let mut out = Preview {
        order_id: String::new(),
        source_hash: format!("{:x}", Sha256::digest(source.as_bytes())),
        records: vec![],
        issues: vec![],
    };
    if source.len() > 1_048_576 {
        out.issues.push("ACO exceeds 1 MiB".into());
        return out;
    }
    if !source.is_ascii() {
        out.issues.push("ACO must contain ASCII text".into());
        return out;
    }
    if !source.trim_end().ends_with("//") {
        out.issues.push("last set is not terminated by //".into());
    }
    let mut current: Option<Builder> = None;
    let mut inherited_periods = vec![];
    let mut action = Action::Create;
    let mut msg = false;
    let mut offset = 0;
    for raw in source.split("//") {
        let line = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
        offset = (offset + raw.len() + 2).min(source.len());
        let text = raw.trim().to_uppercase();
        if text.is_empty() {
            continue;
        }
        let fields: Vec<String> = text
            .split('/')
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let tag = fields[0].as_str();
        let f = &fields[1..];
        if ["ACMID", "ACM", "CANCEL"].contains(&tag) {
            if let Some(b) = current.take() {
                out.records.push(finish(b, options));
            }
            let labelled = f
                .iter()
                .find(|s| s.starts_with("NAME:"))
                .map(|s| value(s).to_owned());
            let id = labelled
                .or_else(|| f.get(1).map(|s| value(s).to_owned()))
                .unwrap_or_default();
            let kind = f
                .iter()
                .find(|s| s.starts_with("USE:"))
                .or(f.first())
                .map(|s| value(s))
                .unwrap_or("");
            let kind = match kind {
                "SECTOR" | "SECT" => "sector",
                "CORRTE" | "AIRCOR" | "CORRIDOR" => "corridor",
                "ROZ" | "RESTRICTED" | "BDZ" => "restricted",
                "CAP" | "PATROL" => "patrol",
                other => other,
            };
            let shape = f
                .iter()
                .find(|s| ["POLYGON", "CIRCLE", "CORRIDOR"].contains(&s.as_str()))
                .cloned()
                .unwrap_or("POLYGON".into());
            let mut coords = vec![];
            for s in f.iter().skip(2) {
                if let Ok(p) = parse_coordinate(s) {
                    coords.push(p)
                }
            }
            current = Some(Builder {
                record: Record {
                    external_id: id.clone(),
                    name: id,
                    action: if tag == "CANCEL" {
                        Action::Cancel
                    } else {
                        action
                    },
                    geometry: None,
                    polygon: vec![],
                    floor_m: None,
                    ceiling_m: None,
                    agency: None,
                    kind: kind.into(),
                    controller_role_id: None,
                    periods: vec![],
                    raw: format!("{raw}//"),
                    line,
                    issues: vec![],
                    warnings: vec![],
                    excluded: false,
                },
                coords,
                shape,
                size: None,
                period_sets: inherited_periods.clone(),
                limits: None,
                geometry_errors: vec![],
            });
            continue;
        }
        if let Some(b) = &mut current {
            b.record.raw.push_str(&format!("{raw}//"));
        }
        match tag {
            "MSGID" => {
                msg = f.first().is_some_and(|s| value(s) == "ACO");
                if !msg {
                    out.issues.push("MSGID must identify ACO".into())
                }
            }
            "ACOID" => out.order_id = f.first().map(|s| value(s).to_owned()).unwrap_or_default(),
            "ACTION" | "AMEND" => {
                let a = match f.first().map(|s| value(s)).unwrap_or("") {
                    "CREATE" | "NEW" | "ADD" => Some(Action::Create),
                    "CHANGE" | "CHG" | "AMEND" => Some(Action::Change),
                    "CANCEL" | "DELETE" | "DEL" => Some(Action::Cancel),
                    _ => None,
                };
                if let Some(a) = a {
                    if let Some(b) = &mut current {
                        b.record.action = a
                    } else {
                        action = a
                    }
                } else {
                    out.issues.push(format!("line {line}: unsupported action"));
                }
            }
            "PERIOD" | "APERIOD" => {
                if let Some(b) = &mut current {
                    if b.period_sets == inherited_periods {
                        b.period_sets.clear()
                    }
                    b.period_sets.push(f.to_vec());
                } else {
                    inherited_periods.push(f.to_vec())
                }
            }
            "POLYGON" | "CORRIDOR" | "CIRCLE" => {
                if let Some(b) = &mut current {
                    b.shape = tag.into();
                    b.coords.clear();
                    for s in f {
                        if s.starts_with("RADIUS:") || s.starts_with("WIDTH:") {
                            match length(s) {
                                Ok(v) => b.size = Some(v),
                                Err(e) => b.geometry_errors.push(e),
                            }
                        } else {
                            match parse_coordinate(s) {
                                Ok(p) => b.coords.push(p),
                                Err(e) => match length(s) {
                                    Ok(v) => b.size = Some(v),
                                    Err(_) => b.geometry_errors.push(e),
                                },
                            }
                        }
                    }
                } else {
                    out.issues
                        .push(format!("line {line}: geometry without ACMID"));
                }
            }
            "SIZE" => {
                if let Some(b) = &mut current {
                    match f
                        .first()
                        .ok_or("missing size".into())
                        .and_then(|s| length(s))
                    {
                        Ok(v) => b.size = Some(v),
                        Err(e) => b.geometry_errors.push(e),
                    }
                }
            }
            "EFFLEVEL" => {
                if let Some(b) = &mut current {
                    b.limits = Some(f.to_vec())
                }
            }
            "CONTROLA" | "CONTROL" => {
                if let Some(b) = &mut current {
                    b.record.agency = f.first().map(|s| value(s).to_owned())
                }
            }
            "EXER" | "OPER" | "AMPN" | "NARR" | "RMKS" | "SECURITY" | "DTG" => {}
            _ => {
                let e=format!("line {line}: unsupported set {tag}; explicitly exclude affected record or use a supported form");
                if let Some(b) = &mut current {
                    b.record.issues.push(e)
                } else {
                    out.issues.push(e)
                }
            }
        }
    }
    if let Some(b) = current {
        out.records.push(finish(b, options));
    }
    if !msg {
        out.issues.push("missing ACO MSGID".into())
    }
    if out.order_id.is_empty() {
        out.issues.push("missing ACOID".into())
    }
    if out.records.is_empty() {
        out.issues.push("no airspace records".into())
    }
    let mut ids = std::collections::BTreeSet::new();
    for r in &out.records {
        if r.external_id.is_empty() || !ids.insert(&r.external_id) {
            out.issues
                .push("missing or duplicate airspace identifier".into());
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn opts() -> ImportOptions {
        ImportOptions {
            anchor_utc: "2026-09-01T00:00:00Z".into(),
            anchor_tick: 0,
            year: 2026,
            horizon_end_utc: "2026-09-04T00:00:00Z".into(),
            resolutions: BTreeMap::from([(
                "WEST".into(),
                Resolution {
                    controller_role_id: Some("controller".into()),
                    ..Default::default()
                },
            )]),
        }
    }
    #[test]
    fn coordinates_and_legacy() {
        assert_eq!(
            parse_coordinate("380000N0770000W").unwrap(),
            LatLon {
                latitude_deg: 38.,
                longitude_deg: -77.
            }
        );
        let p=parse("MSGID/ACO/TEST//ACOID/ONE//PERIOD/010000ZSEP/020000ZSEP//ACMID/SECTOR/WEST/380000N0770000W/380000N0760000W/390000N0760000W//EFFLEVEL/0AMSL-10000AMSL//",&opts());
        assert!(p.issues.is_empty());
        assert!(p.records[0].issues.is_empty(), "{:?}", p.records[0].issues);
        assert_eq!(p.records[0].periods[0].end_tick, 86400);
    }
    #[test]
    fn labelled_shapes_and_unresolved_altitude() {
        let p=parse("MSGID/ACO/TEST//ACOID/ONE//ACMID/ACM:ROZ/NAME:WEST/CIRCLE/USE:ROZ//CIRCLE/RADIUS:5NM/380000N0770000W//EFFLEVEL/FLFL:FL100-FL200//APERIOD/DAILY/010000ZSEP/010100ZSEP//",&opts());
        assert!(p.records[0].polygon.len() > 16);
        assert_eq!(p.records[0].periods.len(), 3);
        assert!(p.records[0].issues.iter().any(|s| s.contains("vertical")));
    }
    #[test]
    fn amendments_and_bad_records() {
        let p = parse(
            "MSGID/ACO/T//ACOID/A//ACTION/CANCEL//ACMID/SECTOR/WEST//",
            &opts(),
        );
        assert_eq!(p.records[0].action, Action::Cancel);
        assert!(p.records[0].issues.is_empty());
        assert!(!parse("MSGID/ACO/T//ACOID/A//UNKNOWN/FOO//", &opts())
            .issues
            .is_empty());
        assert!(parse_coordinate("389900N0770000W").is_err());
    }
}
