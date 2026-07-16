use std::{
    collections::{BTreeMap, HashMap},
    io::{Cursor, Read},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use reqwest::{header, Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

pub const AIRPORT_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_REFRESH_MAX_AGE_SECONDS: u64 = 86_400;
const OURAIRPORTS_AIRPORTS_URL: &str =
    "https://davidmegginson.github.io/ourairports-data/airports.csv";
const OURAIRPORTS_RUNWAYS_URL: &str =
    "https://davidmegginson.github.io/ourairports-data/runways.csv";
const FAA_SUBSCRIPTION_URL: &str =
    "https://www.faa.gov/air_traffic/flight_info/aeronav/aero_data/NASR_Subscription/";

#[derive(Debug, Clone)]
pub struct AirportSyncConfig {
    pub cache_dir: PathBuf,
    pub ourairports_airports_url: String,
    pub ourairports_runways_url: String,
    pub faa_subscription_url: String,
    pub faa_apt_url: Option<String>,
}

impl AirportSyncConfig {
    pub fn from_env() -> Self {
        Self {
            cache_dir: std::env::var("AIRPORT_CACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data/cache/airports")),
            ourairports_airports_url: std::env::var("OURAIRPORTS_AIRPORTS_URL")
                .unwrap_or_else(|_| OURAIRPORTS_AIRPORTS_URL.into()),
            ourairports_runways_url: std::env::var("OURAIRPORTS_RUNWAYS_URL")
                .unwrap_or_else(|_| OURAIRPORTS_RUNWAYS_URL.into()),
            faa_subscription_url: std::env::var("FAA_NASR_SUBSCRIPTION_URL")
                .unwrap_or_else(|_| FAA_SUBSCRIPTION_URL.into()),
            faa_apt_url: std::env::var("FAA_NASR_APT_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        }
    }

    pub fn snapshot_path(&self) -> PathBuf {
        self.cache_dir.join("latest.json")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirportCatalogSnapshot {
    pub schema_version: u32,
    pub synced_unix: u64,
    pub checksum: String,
    pub sources: Vec<AirportSourceMetadata>,
    pub degraded_sources: Vec<String>,
    pub airports: Vec<Airport>,
}

impl AirportCatalogSnapshot {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.schema_version == AIRPORT_SCHEMA_VERSION,
            "unsupported airport cache schema"
        );
        anyhow::ensure!(!self.checksum.is_empty(), "airport cache checksum is empty");
        anyhow::ensure!(!self.airports.is_empty(), "airport cache has no airports");
        for airport in &self.airports {
            anyhow::ensure!(!airport.id.is_empty(), "airport id is empty");
            anyhow::ensure!(
                airport.latitude_deg.is_finite() && (-90.0..=90.0).contains(&airport.latitude_deg),
                "airport latitude is invalid"
            );
            anyhow::ensure!(
                airport.longitude_deg.is_finite()
                    && (-180.0..=180.0).contains(&airport.longitude_deg),
                "airport longitude is invalid"
            );
            for runway in &airport.runways {
                anyhow::ensure!(
                    runway.length_m.is_none_or(|value| value > 0.0),
                    "runway length must be positive"
                );
                anyhow::ensure!(
                    runway.width_m.is_none_or(|value| value > 0.0),
                    "runway width must be positive"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirportSourceMetadata {
    pub id: String,
    pub url: String,
    pub retrieved_unix: u64,
    pub effective_cycle: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub sha256: String,
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Airport {
    pub id: String,
    pub name: String,
    pub kind: AirportKind,
    pub status: FacilityStatus,
    pub latitude_deg: f64,
    pub longitude_deg: f64,
    pub elevation_m: Option<f64>,
    pub country_code: String,
    pub region_code: Option<String>,
    pub municipality: Option<String>,
    pub identifiers: AirportIdentifiers,
    pub ownership_type: Option<String>,
    pub facility_use: Option<String>,
    pub military_use: MilitaryUse,
    pub joint_use: Option<bool>,
    pub military_landing_rights: Option<bool>,
    pub source_ids: Vec<String>,
    pub runways: Vec<Runway>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AirportIdentifiers {
    pub ourairports_ident: Option<String>,
    pub icao: Option<String>,
    pub iata: Option<String>,
    pub gps: Option<String>,
    pub local: Option<String>,
    pub faa_site_number: Option<String>,
    pub faa_airport_id: Option<String>,
}

impl AirportIdentifiers {
    pub fn values(&self) -> impl Iterator<Item = &str> {
        [
            self.ourairports_ident.as_deref(),
            self.icao.as_deref(),
            self.iata.as_deref(),
            self.gps.as_deref(),
            self.local.as_deref(),
            self.faa_airport_id.as_deref(),
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirportKind {
    LargeAirport,
    MediumAirport,
    SmallAirport,
    Heliport,
    SeaplaneBase,
    Balloonport,
    ClosedAirport,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityStatus {
    Open,
    Closed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilitaryUse {
    Military,
    Joint,
    Civilian,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runway {
    pub id: String,
    pub designator: String,
    pub length_m: Option<f64>,
    pub width_m: Option<f64>,
    pub surface: RunwaySurface,
    pub surface_raw: Option<String>,
    pub status: FacilityStatus,
    pub lighted: Option<bool>,
    pub condition: Option<String>,
    pub pavement: Option<PavementClassification>,
    pub gross_weight_limits: GrossWeightLimits,
    pub ends: Vec<RunwayEnd>,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunwaySurface {
    Asphalt,
    Concrete,
    Paved,
    Gravel,
    Grass,
    Dirt,
    Water,
    SnowIce,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayEnd {
    pub designator: String,
    pub latitude_deg: Option<f64>,
    pub longitude_deg: Option<f64>,
    pub elevation_m: Option<f64>,
    pub displaced_threshold_m: Option<f64>,
    pub takeoff_run_available_m: Option<f64>,
    pub takeoff_distance_available_m: Option<f64>,
    pub accelerate_stop_distance_available_m: Option<f64>,
    pub landing_distance_available_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PavementClassification {
    pub system: PavementRatingSystem,
    pub value: f64,
    pub pavement_type: Option<String>,
    pub subgrade_strength: Option<String>,
    pub tire_pressure: Option<String>,
    pub determination_method: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PavementRatingSystem {
    AcnPcn,
    AcrPcr,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GrossWeightLimits {
    pub single_wheel_kg: Option<f64>,
    pub dual_wheel_kg: Option<f64>,
    pub dual_tandem_kg: Option<f64>,
    pub double_dual_tandem_kg: Option<f64>,
}

impl GrossWeightLimits {
    fn for_gear(&self, gear: LandingGearCategory) -> Option<f64> {
        match gear {
            LandingGearCategory::SingleWheel => self.single_wheel_kg,
            LandingGearCategory::DualWheel => self.dual_wheel_kg,
            LandingGearCategory::DualTandem => self.dual_tandem_kg,
            LandingGearCategory::DoubleDualTandem => self.double_dual_tandem_kg,
            LandingGearCategory::Other => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayCompatibilityRequest {
    pub operation: RunwayOperation,
    pub required_distance_m: f64,
    pub aircraft_mass_kg: f64,
    pub landing_gear: Option<LandingGearCategory>,
    pub minimum_width_m: Option<f64>,
    #[serde(default)]
    pub allowed_surfaces: Vec<RunwaySurface>,
    pub aircraft_pavement_rating: Option<AircraftPavementRating>,
}

impl RunwayCompatibilityRequest {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.required_distance_m.is_finite() && self.required_distance_m > 0.0,
            "required distance must be positive"
        );
        anyhow::ensure!(
            self.aircraft_mass_kg.is_finite() && self.aircraft_mass_kg > 0.0,
            "aircraft mass must be positive"
        );
        anyhow::ensure!(
            self.minimum_width_m
                .is_none_or(|value| value.is_finite() && value > 0.0),
            "minimum width must be positive"
        );
        anyhow::ensure!(
            self.aircraft_pavement_rating
                .as_ref()
                .is_none_or(|rating| rating.value.is_finite() && rating.value > 0.0),
            "aircraft pavement rating must be positive"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunwayOperation {
    Takeoff,
    Landing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandingGearCategory {
    SingleWheel,
    DualWheel,
    DualTandem,
    DoubleDualTandem,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AircraftPavementRating {
    pub system: PavementRatingSystem,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayCompatibilityAssessment {
    pub runway_id: String,
    pub runway_end: String,
    pub verdict: CompatibilityVerdict,
    pub available_distance_m: Option<f64>,
    pub applicable_weight_limit_kg: Option<f64>,
    pub reasons: Vec<CompatibilityReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityVerdict {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReason {
    pub code: String,
    pub detail: String,
}

pub fn evaluate_airport(
    airport: &Airport,
    request: &RunwayCompatibilityRequest,
) -> Result<Vec<RunwayCompatibilityAssessment>> {
    request.validate()?;
    let mut assessments = Vec::new();
    for runway in &airport.runways {
        let ends: Vec<RunwayEnd> = if runway.ends.is_empty() {
            vec![RunwayEnd {
                designator: runway.designator.clone(),
                latitude_deg: None,
                longitude_deg: None,
                elevation_m: None,
                displaced_threshold_m: None,
                takeoff_run_available_m: None,
                takeoff_distance_available_m: None,
                accelerate_stop_distance_available_m: None,
                landing_distance_available_m: None,
            }]
        } else {
            runway.ends.clone()
        };
        for end in ends {
            let mut assessment = evaluate_runway_end(runway, &end, request);
            if airport.status == FacilityStatus::Closed {
                assessment.verdict = CompatibilityVerdict::Incompatible;
                reason(
                    &mut assessment.reasons,
                    "airport_closed",
                    "airport is reported closed",
                );
            }
            assessments.push(assessment);
        }
    }
    Ok(assessments)
}

fn evaluate_runway_end(
    runway: &Runway,
    end: &RunwayEnd,
    request: &RunwayCompatibilityRequest,
) -> RunwayCompatibilityAssessment {
    let mut incompatible = false;
    let mut unknown = false;
    let mut reasons = Vec::new();
    if runway.status == FacilityStatus::Closed {
        incompatible = true;
        reason(&mut reasons, "runway_closed", "runway is reported closed");
    }
    let available_distance = match request.operation {
        RunwayOperation::Takeoff => end.takeoff_run_available_m.or(runway.length_m),
        RunwayOperation::Landing => end.landing_distance_available_m.or_else(|| {
            runway
                .length_m
                .map(|length| (length - end.displaced_threshold_m.unwrap_or(0.0)).max(0.0))
        }),
    };
    match available_distance {
        Some(value) if value < request.required_distance_m => {
            incompatible = true;
            reason(
                &mut reasons,
                "insufficient_distance",
                format!(
                    "{value:.1} m available; {:.1} m required",
                    request.required_distance_m
                ),
            );
        }
        None => {
            unknown = true;
            reason(
                &mut reasons,
                "distance_unknown",
                "runway distance is not reported",
            );
        }
        _ => {}
    }
    if let Some(required) = request.minimum_width_m {
        match runway.width_m {
            Some(width) if width < required => {
                incompatible = true;
                reason(
                    &mut reasons,
                    "insufficient_width",
                    format!("{width:.1} m available; {required:.1} m required"),
                );
            }
            None => {
                unknown = true;
                reason(
                    &mut reasons,
                    "width_unknown",
                    "runway width is not reported",
                );
            }
            _ => {}
        }
    }
    if !request.allowed_surfaces.is_empty() {
        if runway.surface == RunwaySurface::Unknown {
            unknown = true;
            reason(
                &mut reasons,
                "surface_unknown",
                "runway surface is not reported",
            );
        } else if !request.allowed_surfaces.contains(&runway.surface) {
            incompatible = true;
            reason(
                &mut reasons,
                "surface_not_allowed",
                format!("{:?} surface is not allowed", runway.surface),
            );
        }
    }
    let weight_limit = request
        .landing_gear
        .and_then(|gear| runway.gross_weight_limits.for_gear(gear));
    let mut load_checked = false;
    if let Some(limit) = weight_limit {
        load_checked = true;
        if request.aircraft_mass_kg > limit {
            incompatible = true;
            reason(
                &mut reasons,
                "weight_limit_exceeded",
                format!(
                    "aircraft mass {:.1} kg exceeds {:.1} kg limit",
                    request.aircraft_mass_kg, limit
                ),
            );
        }
    }
    if let (Some(aircraft), Some(pavement)) = (&request.aircraft_pavement_rating, &runway.pavement)
    {
        if aircraft.system == pavement.system {
            load_checked = true;
            if aircraft.value > pavement.value {
                incompatible = true;
                reason(
                    &mut reasons,
                    "pavement_rating_exceeded",
                    format!(
                        "aircraft rating {:.1} exceeds pavement rating {:.1}",
                        aircraft.value, pavement.value
                    ),
                );
            }
        } else {
            unknown = true;
            reason(
                &mut reasons,
                "pavement_system_mismatch",
                "aircraft and runway pavement rating systems differ",
            );
        }
    }
    if !load_checked {
        unknown = true;
        reason(
            &mut reasons,
            "strength_unknown",
            "no comparable runway strength value is available",
        );
    }
    RunwayCompatibilityAssessment {
        runway_id: runway.id.clone(),
        runway_end: end.designator.clone(),
        verdict: if incompatible {
            CompatibilityVerdict::Incompatible
        } else if unknown {
            CompatibilityVerdict::Unknown
        } else {
            CompatibilityVerdict::Compatible
        },
        available_distance_m: available_distance,
        applicable_weight_limit_kg: weight_limit,
        reasons,
    }
}

fn reason(reasons: &mut Vec<CompatibilityReason>, code: &str, detail: impl Into<String>) {
    reasons.push(CompatibilityReason {
        code: code.into(),
        detail: detail.into(),
    });
}

pub async fn load_airport_snapshot(
    config: &AirportSyncConfig,
) -> Result<Option<AirportCatalogSnapshot>> {
    match tokio::fs::read(config.snapshot_path()).await {
        Ok(bytes) => {
            let snapshot: AirportCatalogSnapshot = serde_json::from_slice(&bytes)?;
            snapshot.validate()?;
            Ok(Some(snapshot))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub async fn sync_airport_snapshot(
    config: &AirportSyncConfig,
    previous: Option<&AirportCatalogSnapshot>,
) -> Result<AirportCatalogSnapshot> {
    let client = Client::builder()
        .user_agent("world-at-war/0.1 airport catalog")
        .build()?;
    let previous_airports = previous.and_then(|snapshot| {
        snapshot
            .sources
            .iter()
            .find(|source| source.id == "ourairports_airports")
    });
    let previous_runways = previous.and_then(|snapshot| {
        snapshot
            .sources
            .iter()
            .find(|source| source.id == "ourairports_runways")
    });
    let airports_path = config.cache_dir.join("raw/ourairports/airports.csv");
    let runways_path = config.cache_dir.join("raw/ourairports/runways.csv");
    let (airports_download, runways_download) = tokio::try_join!(
        fetch_source(
            &client,
            "ourairports_airports",
            &config.ourairports_airports_url,
            &airports_path,
            "Public Domain",
            previous_airports
        ),
        fetch_source(
            &client,
            "ourairports_runways",
            &config.ourairports_runways_url,
            &runways_path,
            "Public Domain",
            previous_runways
        )
    )?;
    let mut airports = parse_ourairports(&airports_download.bytes, &runways_download.bytes)?;
    let mut sources = vec![airports_download.metadata, runways_download.metadata];
    let mut degraded_sources = Vec::new();
    match fetch_and_parse_faa(&client, config, previous).await {
        Ok((faa, metadata)) => {
            overlay_faa(&mut airports, faa);
            sources.push(metadata);
        }
        Err(error)
            if previous.is_none_or(|snapshot| {
                !snapshot
                    .sources
                    .iter()
                    .any(|source| source.id == "faa_nasr")
            }) =>
        {
            degraded_sources.push(format!("faa_nasr: {error}"));
        }
        Err(error) => {
            return Err(error
                .context("FAA refresh failed; retaining the previous complete airport snapshot"))
        }
    }
    airports.sort_by(|left, right| left.id.cmp(&right.id));
    for airport in &mut airports {
        airport
            .runways
            .sort_by(|left, right| left.id.cmp(&right.id));
    }
    let checksum = format!("{:x}", Sha256::digest(serde_json::to_vec(&airports)?));
    let snapshot = AirportCatalogSnapshot {
        schema_version: AIRPORT_SCHEMA_VERSION,
        synced_unix: now_unix(),
        checksum,
        sources,
        degraded_sources,
        airports,
    };
    snapshot.validate()?;
    persist_snapshot(config, &snapshot).await?;
    Ok(snapshot)
}

struct DownloadedSource {
    bytes: Vec<u8>,
    metadata: AirportSourceMetadata,
}

async fn fetch_source(
    client: &Client,
    id: &str,
    url: &str,
    path: &Path,
    license: &str,
    previous: Option<&AirportSourceMetadata>,
) -> Result<DownloadedSource> {
    let mut request = client.get(url);
    if let Some(etag) = previous.and_then(|source| source.etag.as_ref()) {
        request = request.header(header::IF_NONE_MATCH, etag);
    }
    if let Some(modified) = previous.and_then(|source| source.last_modified.as_ref()) {
        request = request.header(header::IF_MODIFIED_SINCE, modified);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("could not fetch {id}"))?;
    if response.status() == StatusCode::NOT_MODIFIED {
        let bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("{id} returned 304 but cached raw data is missing"))?;
        let mut metadata = previous
            .cloned()
            .context("conditional response has no prior metadata")?;
        metadata.retrieved_unix = now_unix();
        return Ok(DownloadedSource { bytes, metadata });
    }
    anyhow::ensure!(
        response.status().is_success(),
        "{id} returned HTTP {}",
        response.status()
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let last_modified = response
        .headers()
        .get(header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response.bytes().await?.to_vec();
    anyhow::ensure!(!bytes.is_empty(), "{id} returned an empty response");
    atomic_write(path, &bytes).await?;
    Ok(DownloadedSource {
        metadata: AirportSourceMetadata {
            id: id.into(),
            url: url.into(),
            retrieved_unix: now_unix(),
            effective_cycle: None,
            etag,
            last_modified,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            license: license.into(),
        },
        bytes,
    })
}

async fn fetch_and_parse_faa(
    client: &Client,
    config: &AirportSyncConfig,
    previous: Option<&AirportCatalogSnapshot>,
) -> Result<(FaaCatalog, AirportSourceMetadata)> {
    let apt_url = match &config.faa_apt_url {
        Some(url) => url.clone(),
        None => discover_faa_apt_url(client, &config.faa_subscription_url).await?,
    };
    let previous_source = previous.and_then(|snapshot| {
        snapshot
            .sources
            .iter()
            .find(|source| source.id == "faa_nasr" && source.url == apt_url)
    });
    let download = fetch_source(
        client,
        "faa_nasr",
        &apt_url,
        &config.cache_dir.join("raw/faa/APT_CSV.zip"),
        "U.S. Government public data",
        previous_source,
    )
    .await?;
    let faa = parse_faa_zip(&download.bytes)?;
    let mut metadata = download.metadata;
    metadata.effective_cycle = faa.effective_cycle.clone();
    Ok((faa, metadata))
}

async fn discover_faa_apt_url(client: &Client, subscription_url: &str) -> Result<String> {
    let html = client
        .get(subscription_url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let current_start = html
        .find(">Current<")
        .context("FAA subscription page has no Current section")?;
    let current_end = html[current_start..]
        .find(">Archives<")
        .map(|offset| current_start + offset)
        .unwrap_or(html.len());
    let cycle_href = extract_hrefs(&html[current_start..current_end])
        .into_iter()
        .find(|href| {
            href.contains("NASR_Subscription/") && href.chars().filter(|ch| *ch == '-').count() >= 2
        })
        .context("FAA current subscription link was not found")?;
    let cycle_url = Url::parse(subscription_url)?.join(&cycle_href)?;
    let cycle_html = client
        .get(cycle_url.clone())
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let apt_href = extract_hrefs(&cycle_html)
        .into_iter()
        .find(|href| href.to_ascii_uppercase().contains("_APT_CSV.ZIP"))
        .context("FAA APT CSV archive link was not found")?;
    Ok(cycle_url.join(&apt_href)?.to_string())
}

fn extract_hrefs(html: &str) -> Vec<String> {
    let mut hrefs = Vec::new();
    let mut rest = html;
    while let Some(index) = rest.find("href=\"") {
        rest = &rest[index + 6..];
        let Some(end) = rest.find('"') else { break };
        hrefs.push(rest[..end].replace("&amp;", "&"));
        rest = &rest[end + 1..];
    }
    hrefs
}

async fn persist_snapshot(
    config: &AirportSyncConfig,
    snapshot: &AirportCatalogSnapshot,
) -> Result<()> {
    atomic_write(&config.snapshot_path(), &serde_json::to_vec(snapshot)?).await
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let temporary = path.with_extension("tmp");
    tokio::fs::write(&temporary, bytes).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}

#[derive(Deserialize)]
struct OurAirportRow {
    id: String,
    ident: String,
    #[serde(rename = "type")]
    kind: String,
    name: String,
    latitude_deg: f64,
    longitude_deg: f64,
    elevation_ft: Option<f64>,
    iso_country: String,
    iso_region: Option<String>,
    municipality: Option<String>,
    gps_code: Option<String>,
    icao_code: Option<String>,
    iata_code: Option<String>,
    local_code: Option<String>,
}

#[derive(Deserialize)]
struct OurRunwayRow {
    id: String,
    airport_ref: String,
    length_ft: Option<f64>,
    width_ft: Option<f64>,
    surface: Option<String>,
    lighted: Option<u8>,
    closed: Option<u8>,
    le_ident: Option<String>,
    le_latitude_deg: Option<f64>,
    le_longitude_deg: Option<f64>,
    le_elevation_ft: Option<f64>,
    le_displaced_threshold_ft: Option<f64>,
    he_ident: Option<String>,
    he_latitude_deg: Option<f64>,
    he_longitude_deg: Option<f64>,
    he_elevation_ft: Option<f64>,
    he_displaced_threshold_ft: Option<f64>,
}

pub fn parse_ourairports(airport_csv: &[u8], runway_csv: &[u8]) -> Result<Vec<Airport>> {
    let mut airports = Vec::new();
    let mut by_ref = HashMap::new();
    for row in csv::Reader::from_reader(airport_csv).deserialize::<OurAirportRow>() {
        let row = row?;
        let index = airports.len();
        by_ref.insert(row.id.clone(), index);
        let closed = row.kind == "closed_airport";
        airports.push(Airport {
            id: format!("ourairports:{}", row.id),
            name: row.name,
            kind: parse_airport_kind(&row.kind),
            status: if closed {
                FacilityStatus::Closed
            } else {
                FacilityStatus::Open
            },
            latitude_deg: row.latitude_deg,
            longitude_deg: row.longitude_deg,
            elevation_m: feet_to_meters(row.elevation_ft),
            country_code: row.iso_country,
            region_code: row.iso_region,
            municipality: row.municipality,
            identifiers: AirportIdentifiers {
                ourairports_ident: nonempty(row.ident),
                icao: row.icao_code.and_then(nonempty),
                iata: row.iata_code.and_then(nonempty),
                gps: row.gps_code.and_then(nonempty),
                local: row.local_code.and_then(nonempty),
                ..Default::default()
            },
            ownership_type: None,
            facility_use: None,
            military_use: MilitaryUse::Unknown,
            joint_use: None,
            military_landing_rights: None,
            source_ids: vec!["ourairports_airports".into()],
            runways: Vec::new(),
        });
    }
    for row in csv::Reader::from_reader(runway_csv).deserialize::<OurRunwayRow>() {
        let row = row?;
        let Some(index) = by_ref.get(&row.airport_ref).copied() else {
            continue;
        };
        let mut ends = Vec::new();
        if let Some(designator) = row.le_ident.clone().and_then(nonempty) {
            ends.push(RunwayEnd {
                designator,
                latitude_deg: row.le_latitude_deg,
                longitude_deg: row.le_longitude_deg,
                elevation_m: feet_to_meters(row.le_elevation_ft),
                displaced_threshold_m: feet_to_meters(row.le_displaced_threshold_ft),
                takeoff_run_available_m: None,
                takeoff_distance_available_m: None,
                accelerate_stop_distance_available_m: None,
                landing_distance_available_m: None,
            });
        }
        if let Some(designator) = row.he_ident.clone().and_then(nonempty) {
            ends.push(RunwayEnd {
                designator,
                latitude_deg: row.he_latitude_deg,
                longitude_deg: row.he_longitude_deg,
                elevation_m: feet_to_meters(row.he_elevation_ft),
                displaced_threshold_m: feet_to_meters(row.he_displaced_threshold_ft),
                takeoff_run_available_m: None,
                takeoff_distance_available_m: None,
                accelerate_stop_distance_available_m: None,
                landing_distance_available_m: None,
            });
        }
        let designator = ends
            .iter()
            .map(|end| end.designator.as_str())
            .collect::<Vec<_>>()
            .join("/");
        let surface_raw = row.surface.and_then(nonempty);
        let runway = Runway {
            id: format!("ourairports:{}", row.id),
            designator,
            length_m: positive_feet_to_meters(row.length_ft),
            width_m: positive_feet_to_meters(row.width_ft),
            surface: parse_surface(surface_raw.as_deref()),
            surface_raw,
            status: if row.closed == Some(1) {
                FacilityStatus::Closed
            } else {
                FacilityStatus::Open
            },
            lighted: row.lighted.map(|value| value == 1),
            condition: None,
            pavement: None,
            gross_weight_limits: GrossWeightLimits::default(),
            ends,
            source_ids: vec!["ourairports_runways".into()],
        };
        let key = normalize_designator(&runway.designator);
        if let Some(existing) = airports[index]
            .runways
            .iter_mut()
            .find(|existing| normalize_designator(&existing.designator) == key)
        {
            merge_duplicate_runway(existing, runway);
        } else {
            airports[index].runways.push(runway);
        }
    }
    Ok(airports)
}

fn merge_duplicate_runway(existing: &mut Runway, duplicate: Runway) {
    existing.length_m = max_option(existing.length_m, duplicate.length_m);
    existing.width_m = max_option(existing.width_m, duplicate.width_m);
    if existing.surface == RunwaySurface::Unknown && duplicate.surface != RunwaySurface::Unknown {
        existing.surface = duplicate.surface;
        existing.surface_raw = duplicate.surface_raw;
    }
    if duplicate.status == FacilityStatus::Open {
        existing.status = FacilityStatus::Open;
    }
    existing.lighted = match (existing.lighted, duplicate.lighted) {
        (Some(left), Some(right)) => Some(left || right),
        (left, right) => left.or(right),
    };
    for duplicate_end in duplicate.ends {
        let key = normalize_designator(&duplicate_end.designator);
        if let Some(existing_end) = existing
            .ends
            .iter_mut()
            .find(|end| normalize_designator(&end.designator) == key)
        {
            existing_end.latitude_deg = existing_end.latitude_deg.or(duplicate_end.latitude_deg);
            existing_end.longitude_deg = existing_end.longitude_deg.or(duplicate_end.longitude_deg);
            existing_end.elevation_m = existing_end.elevation_m.or(duplicate_end.elevation_m);
            existing_end.displaced_threshold_m = existing_end
                .displaced_threshold_m
                .or(duplicate_end.displaced_threshold_m);
        } else {
            existing.ends.push(duplicate_end);
        }
    }
    for source_id in duplicate.source_ids {
        push_unique(&mut existing.source_ids, &source_id);
    }
}

fn max_option(left: Option<f64>, right: Option<f64>) -> Option<f64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (left, right) => left.or(right),
    }
}

struct FaaCatalog {
    effective_cycle: Option<String>,
    bases: Vec<FaaBase>,
    runways: Vec<FaaRunway>,
    ends: Vec<FaaEnd>,
}

#[derive(Deserialize)]
struct FaaBase {
    #[serde(rename = "EFF_DATE")]
    effective_date: String,
    #[serde(rename = "SITE_NO")]
    site_number: String,
    #[serde(rename = "SITE_TYPE_CODE")]
    site_type: String,
    #[serde(rename = "ARPT_ID")]
    airport_id: String,
    #[serde(rename = "ARPT_NAME")]
    name: String,
    #[serde(rename = "COUNTRY_CODE")]
    country_code: String,
    #[serde(rename = "STATE_CODE")]
    state_code: Option<String>,
    #[serde(rename = "CITY")]
    city: String,
    #[serde(rename = "LAT_DECIMAL")]
    latitude_deg: f64,
    #[serde(rename = "LONG_DECIMAL")]
    longitude_deg: f64,
    #[serde(rename = "ELEV")]
    elevation_ft: Option<f64>,
    #[serde(rename = "ARPT_STATUS")]
    status: String,
    #[serde(rename = "OWNERSHIP_TYPE_CODE")]
    ownership_type: Option<String>,
    #[serde(rename = "FACILITY_USE_CODE")]
    facility_use: Option<String>,
    #[serde(rename = "JOINT_USE_FLAG")]
    joint_use: Option<String>,
    #[serde(rename = "MIL_LNDG_FLAG")]
    military_landing: Option<String>,
    #[serde(rename = "ICAO_ID")]
    icao: Option<String>,
}

#[derive(Deserialize)]
struct FaaRunway {
    #[serde(rename = "SITE_NO")]
    site_number: String,
    #[serde(rename = "RWY_ID")]
    runway_id: String,
    #[serde(rename = "RWY_LEN")]
    length_ft: Option<f64>,
    #[serde(rename = "RWY_WIDTH")]
    width_ft: Option<f64>,
    #[serde(rename = "SURFACE_TYPE_CODE")]
    surface: Option<String>,
    #[serde(rename = "COND")]
    condition: Option<String>,
    #[serde(rename = "PCN")]
    pcn: Option<f64>,
    #[serde(rename = "PAVEMENT_TYPE_CODE")]
    pavement_type: Option<String>,
    #[serde(rename = "SUBGRADE_STRENGTH_CODE")]
    subgrade: Option<String>,
    #[serde(rename = "TIRE_PRES_CODE")]
    tire_pressure: Option<String>,
    #[serde(rename = "DTRM_METHOD_CODE")]
    determination_method: Option<String>,
    #[serde(rename = "RWY_LGT_CODE")]
    lighting: Option<String>,
    #[serde(rename = "GROSS_WT_SW")]
    single_wheel_thousand_lb: Option<f64>,
    #[serde(rename = "GROSS_WT_DW")]
    dual_wheel_thousand_lb: Option<f64>,
    #[serde(rename = "GROSS_WT_DTW")]
    dual_tandem_thousand_lb: Option<f64>,
    #[serde(rename = "GROSS_WT_DDTW")]
    double_dual_tandem_thousand_lb: Option<f64>,
}

#[derive(Deserialize)]
struct FaaEnd {
    #[serde(rename = "SITE_NO")]
    site_number: String,
    #[serde(rename = "RWY_ID")]
    runway_id: String,
    #[serde(rename = "RWY_END_ID")]
    runway_end_id: String,
    #[serde(rename = "LAT_DECIMAL")]
    latitude_deg: Option<f64>,
    #[serde(rename = "LONG_DECIMAL")]
    longitude_deg: Option<f64>,
    #[serde(rename = "RWY_END_ELEV")]
    elevation_ft: Option<f64>,
    #[serde(rename = "DISPLACED_THR_LEN")]
    displaced_threshold_ft: Option<f64>,
    #[serde(rename = "TKOF_RUN_AVBL")]
    takeoff_run_ft: Option<f64>,
    #[serde(rename = "TKOF_DIST_AVBL")]
    takeoff_distance_ft: Option<f64>,
    #[serde(rename = "ACLT_STOP_DIST_AVBL")]
    accelerate_stop_ft: Option<f64>,
    #[serde(rename = "LNDG_DIST_AVBL")]
    landing_distance_ft: Option<f64>,
}

fn parse_faa_zip(bytes: &[u8]) -> Result<FaaCatalog> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let bases = read_zip_entry(&mut archive, "APT_BASE.csv")?;
    let runways = read_zip_entry(&mut archive, "APT_RWY.csv")?;
    let ends = read_zip_entry(&mut archive, "APT_RWY_END.csv")?;
    let bases: Vec<FaaBase> = csv::Reader::from_reader(bases.as_bytes())
        .deserialize()
        .collect::<std::result::Result<_, _>>()?;
    let effective_cycle = bases.first().map(|base| base.effective_date.clone());
    Ok(FaaCatalog {
        effective_cycle,
        bases,
        runways: csv::Reader::from_reader(runways.as_bytes())
            .deserialize()
            .collect::<std::result::Result<_, _>>()?,
        ends: csv::Reader::from_reader(ends.as_bytes())
            .deserialize()
            .collect::<std::result::Result<_, _>>()?,
    })
}

fn read_zip_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String> {
    let mut value = String::new();
    archive.by_name(name)?.read_to_string(&mut value)?;
    Ok(value)
}

fn overlay_faa(airports: &mut Vec<Airport>, faa: FaaCatalog) {
    let mut icao_index: HashMap<String, Option<usize>> = HashMap::new();
    let mut country_identifier_index: HashMap<(String, String), Option<usize>> = HashMap::new();
    for (index, airport) in airports.iter().enumerate() {
        if let Some(icao) = airport.identifiers.icao.as_deref() {
            insert_unique_identifier(&mut icao_index, icao.trim().to_ascii_uppercase(), index);
        }
        for identifier in [
            airport.identifiers.gps.as_deref(),
            airport.identifiers.local.as_deref(),
            airport.identifiers.ourairports_ident.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            insert_unique_identifier(
                &mut country_identifier_index,
                (
                    airport.country_code.to_ascii_uppercase(),
                    identifier.trim().to_ascii_uppercase(),
                ),
                index,
            );
        }
    }
    let mut by_site = HashMap::new();
    for base in faa.bases {
        let icao = base
            .icao
            .clone()
            .and_then(nonempty)
            .map(|value| value.to_ascii_uppercase());
        let local = base.airport_id.trim().to_ascii_uppercase();
        let matched = icao
            .as_ref()
            .and_then(|value| icao_index.get(value).copied().flatten())
            .or_else(|| {
                country_identifier_index
                    .get(&(base.country_code.to_ascii_uppercase(), local.clone()))
                    .copied()
                    .flatten()
            });
        let index = matched.unwrap_or_else(|| {
            airports.push(Airport {
                id: format!("faa:{}", base.site_number.trim()),
                name: base.name.clone(),
                kind: faa_airport_kind(&base.site_type),
                status: faa_status(&base.status),
                latitude_deg: base.latitude_deg,
                longitude_deg: base.longitude_deg,
                elevation_m: feet_to_meters(base.elevation_ft),
                country_code: base.country_code.clone(),
                region_code: base
                    .state_code
                    .clone()
                    .and_then(nonempty)
                    .map(|state| format!("US-{state}")),
                municipality: nonempty(base.city.clone()),
                identifiers: AirportIdentifiers::default(),
                ownership_type: None,
                facility_use: None,
                military_use: MilitaryUse::Unknown,
                joint_use: None,
                military_landing_rights: None,
                source_ids: Vec::new(),
                runways: Vec::new(),
            });
            airports.len() - 1
        });
        let airport = &mut airports[index];
        airport.name = base.name;
        airport.status = faa_status(&base.status);
        airport.latitude_deg = base.latitude_deg;
        airport.longitude_deg = base.longitude_deg;
        airport.elevation_m = feet_to_meters(base.elevation_ft).or(airport.elevation_m);
        airport.ownership_type = base.ownership_type.and_then(nonempty);
        airport.facility_use = base.facility_use.and_then(nonempty);
        airport.joint_use = yes_no(base.joint_use.as_deref());
        airport.military_landing_rights = yes_no(base.military_landing.as_deref());
        airport.military_use = if airport.joint_use == Some(true) {
            MilitaryUse::Joint
        } else if airport
            .ownership_type
            .as_deref()
            .is_some_and(|value| value.starts_with('M'))
        {
            MilitaryUse::Military
        } else {
            MilitaryUse::Civilian
        };
        airport.identifiers.icao = icao.or(airport.identifiers.icao.clone());
        airport.identifiers.faa_site_number = Some(base.site_number.trim().into());
        airport.identifiers.faa_airport_id = Some(local);
        push_unique(&mut airport.source_ids, "faa_nasr");
        by_site.insert(base.site_number.trim().to_owned(), index);
    }
    let mut runway_index = HashMap::new();
    for row in faa.runways {
        let site = row.site_number.trim().to_owned();
        let Some(airport_index) = by_site.get(&site).copied() else {
            continue;
        };
        let key = normalize_designator(&row.runway_id);
        let existing = airports[airport_index]
            .runways
            .iter()
            .position(|runway| normalize_designator(&runway.designator) == key);
        let runway_position = existing.unwrap_or_else(|| {
            airports[airport_index].runways.push(Runway {
                id: format!("faa:{site}:{key}"),
                designator: row.runway_id.clone(),
                length_m: None,
                width_m: None,
                surface: RunwaySurface::Unknown,
                surface_raw: None,
                status: FacilityStatus::Open,
                lighted: None,
                condition: None,
                pavement: None,
                gross_weight_limits: GrossWeightLimits::default(),
                ends: Vec::new(),
                source_ids: Vec::new(),
            });
            airports[airport_index].runways.len() - 1
        });
        let runway = &mut airports[airport_index].runways[runway_position];
        runway.designator = row.runway_id;
        runway.length_m = positive_feet_to_meters(row.length_ft).or(runway.length_m);
        runway.width_m = positive_feet_to_meters(row.width_ft).or(runway.width_m);
        runway.surface_raw = row
            .surface
            .and_then(nonempty)
            .or(runway.surface_raw.clone());
        runway.surface = parse_surface(runway.surface_raw.as_deref());
        runway.condition = row.condition.and_then(nonempty);
        runway.lighted = row
            .lighting
            .as_deref()
            .map(|value| !value.trim().is_empty());
        runway.pavement =
            row.pcn
                .filter(|value| *value > 0.0)
                .map(|value| PavementClassification {
                    system: PavementRatingSystem::AcnPcn,
                    value,
                    pavement_type: row.pavement_type.and_then(nonempty),
                    subgrade_strength: row.subgrade.and_then(nonempty),
                    tire_pressure: row.tire_pressure.and_then(nonempty),
                    determination_method: row.determination_method.and_then(nonempty),
                });
        runway.gross_weight_limits = GrossWeightLimits {
            single_wheel_kg: thousand_pounds_to_kg(row.single_wheel_thousand_lb),
            dual_wheel_kg: thousand_pounds_to_kg(row.dual_wheel_thousand_lb),
            dual_tandem_kg: thousand_pounds_to_kg(row.dual_tandem_thousand_lb),
            double_dual_tandem_kg: thousand_pounds_to_kg(row.double_dual_tandem_thousand_lb),
        };
        push_unique(&mut runway.source_ids, "faa_nasr");
        runway_index.insert((site, key), (airport_index, runway_position));
    }
    for row in faa.ends {
        let key = (
            row.site_number.trim().to_owned(),
            normalize_designator(&row.runway_id),
        );
        let Some((airport_index, runway_position)) = runway_index.get(&key).copied() else {
            continue;
        };
        let runway = &mut airports[airport_index].runways[runway_position];
        let designator = row.runway_end_id.trim().to_owned();
        let end = runway
            .ends
            .iter_mut()
            .find(|end| end.designator.eq_ignore_ascii_case(&designator));
        let value = RunwayEnd {
            designator,
            latitude_deg: row.latitude_deg,
            longitude_deg: row.longitude_deg,
            elevation_m: feet_to_meters(row.elevation_ft),
            displaced_threshold_m: feet_to_meters(row.displaced_threshold_ft),
            takeoff_run_available_m: feet_to_meters(row.takeoff_run_ft),
            takeoff_distance_available_m: feet_to_meters(row.takeoff_distance_ft),
            accelerate_stop_distance_available_m: feet_to_meters(row.accelerate_stop_ft),
            landing_distance_available_m: feet_to_meters(row.landing_distance_ft),
        };
        if let Some(end) = end {
            *end = value;
        } else {
            runway.ends.push(value);
        }
    }
}

fn insert_unique_identifier<K: std::hash::Hash + Eq>(
    index: &mut HashMap<K, Option<usize>>,
    key: K,
    airport_index: usize,
) {
    index
        .entry(key)
        .and_modify(|value| {
            if *value != Some(airport_index) {
                *value = None;
            }
        })
        .or_insert(Some(airport_index));
}

fn parse_airport_kind(value: &str) -> AirportKind {
    match value {
        "large_airport" => AirportKind::LargeAirport,
        "medium_airport" => AirportKind::MediumAirport,
        "small_airport" => AirportKind::SmallAirport,
        "heliport" => AirportKind::Heliport,
        "seaplane_base" => AirportKind::SeaplaneBase,
        "balloonport" => AirportKind::Balloonport,
        "closed_airport" => AirportKind::ClosedAirport,
        _ => AirportKind::Unknown,
    }
}

fn faa_airport_kind(value: &str) -> AirportKind {
    match value {
        "H" => AirportKind::Heliport,
        "S" => AirportKind::SeaplaneBase,
        "A" => AirportKind::SmallAirport,
        _ => AirportKind::Unknown,
    }
}

fn faa_status(value: &str) -> FacilityStatus {
    match value.trim() {
        "O" => FacilityStatus::Open,
        "C" | "CI" => FacilityStatus::Closed,
        _ => FacilityStatus::Unknown,
    }
}

fn parse_surface(value: Option<&str>) -> RunwaySurface {
    let value = value.unwrap_or_default().to_ascii_uppercase();
    if value.contains("ASPH") || value == "ASP" {
        RunwaySurface::Asphalt
    } else if value.contains("CONC") || value == "CON" {
        RunwaySurface::Concrete
    } else if value.contains("GRVL") || value.contains("GRAVEL") || value == "GRE" {
        RunwaySurface::Gravel
    } else if value.contains("TURF") || value.contains("GRASS") || value == "GRS" {
        RunwaySurface::Grass
    } else if value.contains("DIRT") || value.contains("EARTH") {
        RunwaySurface::Dirt
    } else if value.contains("WATER") {
        RunwaySurface::Water
    } else if value.contains("SNOW") || value.contains("ICE") {
        RunwaySurface::SnowIce
    } else if value.contains("PAVED") {
        RunwaySurface::Paved
    } else if value.trim().is_empty() || value == "UNK" {
        RunwaySurface::Unknown
    } else {
        RunwaySurface::Other
    }
}

fn normalize_designator(value: &str) -> String {
    let mut parts: Vec<String> = value
        .split(['/', '-'])
        .map(|part| {
            let part = part.trim().to_ascii_uppercase();
            let digit_count = part.chars().take_while(char::is_ascii_digit).count();
            let (number, suffix) = part.split_at(digit_count);
            let number = number.trim_start_matches('0');
            format!("{}{}", if number.is_empty() { "0" } else { number }, suffix)
        })
        .filter(|part| !part.is_empty())
        .collect();
    parts.sort();
    parts.join("/")
}

fn feet_to_meters(value: Option<f64>) -> Option<f64> {
    value.map(|value| value * 0.3048)
}

fn positive_feet_to_meters(value: Option<f64>) -> Option<f64> {
    value
        .filter(|value| *value > 0.0)
        .map(|value| value * 0.3048)
}

fn thousand_pounds_to_kg(value: Option<f64>) -> Option<f64> {
    value
        .filter(|value| *value > 0.0)
        .map(|value| value * 453.59237)
}

fn yes_no(value: Option<&str>) -> Option<bool> {
    match value.map(str::trim) {
        Some("Y") => Some(true),
        Some("N") => Some(false),
        _ => None,
    }
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.into());
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn index_airports(snapshot: &AirportCatalogSnapshot) -> BTreeMap<String, Airport> {
    snapshot
        .airports
        .iter()
        .cloned()
        .map(|airport| (airport.id.clone(), airport))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn airport_with_runway(runway: Runway) -> Airport {
        Airport {
            id: "test:airport".into(),
            name: "Test".into(),
            kind: AirportKind::SmallAirport,
            status: FacilityStatus::Open,
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            elevation_m: None,
            country_code: "US".into(),
            region_code: None,
            municipality: None,
            identifiers: AirportIdentifiers::default(),
            ownership_type: None,
            facility_use: None,
            military_use: MilitaryUse::Unknown,
            joint_use: None,
            military_landing_rights: None,
            source_ids: vec!["test".into()],
            runways: vec![runway],
        }
    }

    fn runway() -> Runway {
        Runway {
            id: "test:runway".into(),
            designator: "09/27".into(),
            length_m: Some(2_000.0),
            width_m: Some(45.0),
            surface: RunwaySurface::Concrete,
            surface_raw: Some("CONC".into()),
            status: FacilityStatus::Open,
            lighted: Some(true),
            condition: Some("GOOD".into()),
            pavement: None,
            gross_weight_limits: GrossWeightLimits {
                dual_wheel_kg: Some(80_000.0),
                ..Default::default()
            },
            ends: vec![RunwayEnd {
                designator: "09".into(),
                latitude_deg: None,
                longitude_deg: None,
                elevation_m: None,
                displaced_threshold_m: Some(200.0),
                takeoff_run_available_m: None,
                takeoff_distance_available_m: None,
                accelerate_stop_distance_available_m: None,
                landing_distance_available_m: None,
            }],
            source_ids: vec!["test".into()],
        }
    }

    fn requirements() -> RunwayCompatibilityRequest {
        RunwayCompatibilityRequest {
            operation: RunwayOperation::Landing,
            required_distance_m: 1_500.0,
            aircraft_mass_kg: 70_000.0,
            landing_gear: Some(LandingGearCategory::DualWheel),
            minimum_width_m: Some(40.0),
            allowed_surfaces: vec![RunwaySurface::Concrete],
            aircraft_pavement_rating: None,
        }
    }

    #[test]
    fn parses_global_airport_and_runway_units() {
        let airports = b"id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,iso_region,municipality,gps_code,icao_code,iata_code,local_code\n1,KAAA,large_airport,Alpha,10,-20,100,US,US-CA,Town,KAAA,KAAA,AAA,AAA\n";
        let runways = b"id,airport_ref,length_ft,width_ft,surface,lighted,closed,le_ident,le_latitude_deg,le_longitude_deg,le_elevation_ft,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_elevation_ft,he_displaced_threshold_ft\n2,1,10000,150,ASPH,1,0,09,10,-20,100,500,27,10.1,-20.1,101,\n";
        let parsed = parse_ourairports(airports, runways).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!((parsed[0].runways[0].length_m.unwrap() - 3_048.0).abs() < 0.01);
        assert_eq!(parsed[0].runways[0].surface, RunwaySurface::Asphalt);
    }

    #[test]
    fn evaluates_distance_weight_and_unknown_strength() {
        let assessment = evaluate_airport(&airport_with_runway(runway()), &requirements()).unwrap();
        assert_eq!(assessment[0].verdict, CompatibilityVerdict::Compatible);
        let mut heavy = requirements();
        heavy.aircraft_mass_kg = 90_000.0;
        assert_eq!(
            evaluate_airport(&airport_with_runway(runway()), &heavy).unwrap()[0].verdict,
            CompatibilityVerdict::Incompatible
        );
        let mut unknown = runway();
        unknown.gross_weight_limits = GrossWeightLimits::default();
        assert_eq!(
            evaluate_airport(&airport_with_runway(unknown), &requirements()).unwrap()[0].verdict,
            CompatibilityVerdict::Unknown
        );
    }

    #[test]
    fn landing_distance_accounts_for_displaced_threshold() {
        let mut request = requirements();
        request.required_distance_m = 1_900.0;
        let assessment = evaluate_airport(&airport_with_runway(runway()), &request).unwrap();
        assert_eq!(assessment[0].available_distance_m, Some(1_800.0));
        assert_eq!(assessment[0].verdict, CompatibilityVerdict::Incompatible);
    }

    #[test]
    fn does_not_mix_pavement_rating_systems() {
        let mut runway = runway();
        runway.gross_weight_limits = GrossWeightLimits::default();
        runway.pavement = Some(PavementClassification {
            system: PavementRatingSystem::AcnPcn,
            value: 50.0,
            pavement_type: None,
            subgrade_strength: None,
            tire_pressure: None,
            determination_method: None,
        });
        let mut request = requirements();
        request.landing_gear = None;
        request.aircraft_pavement_rating = Some(AircraftPavementRating {
            system: PavementRatingSystem::AcrPcr,
            value: 40.0,
        });
        assert_eq!(
            evaluate_airport(&airport_with_runway(runway), &request).unwrap()[0].verdict,
            CompatibilityVerdict::Unknown
        );
    }

    #[test]
    fn faa_overlay_uses_exact_identifiers_and_normalizes_weight() {
        let mut airports = parse_ourairports(
            b"id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,iso_region,municipality,gps_code,icao_code,iata_code,local_code\n1,KAAA,large_airport,Old Name,10,-20,100,US,US-CA,Town,KAAA,KAAA,AAA,AAA\n3,LILA,small_airport,Italian Airport,44,8,200,IT,IT-21,Town,LILA,,,AAA\n",
            b"id,airport_ref,length_ft,width_ft,surface,lighted,closed,le_ident,le_latitude_deg,le_longitude_deg,le_elevation_ft,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_elevation_ft,he_displaced_threshold_ft\n2,1,9000,100,ASPH,1,0,9,10,-20,100,,27,10.1,-20.1,101,\n4,1,9100,110,ASPH,0,0,09,10,-20,100,,27,10.1,-20.1,101,\n",
        ).unwrap();
        overlay_faa(
            &mut airports,
            FaaCatalog {
                effective_cycle: Some("2026/07/09".into()),
                bases: vec![FaaBase {
                    effective_date: "2026/07/09".into(),
                    site_number: "00001.".into(),
                    site_type: "A".into(),
                    airport_id: "AAA".into(),
                    name: "Authoritative Name".into(),
                    country_code: "US".into(),
                    state_code: Some("CA".into()),
                    city: "Town".into(),
                    latitude_deg: 10.0,
                    longitude_deg: -20.0,
                    elevation_ft: Some(100.0),
                    status: "O".into(),
                    ownership_type: Some("PU".into()),
                    facility_use: Some("PU".into()),
                    joint_use: Some("N".into()),
                    military_landing: Some("N".into()),
                    icao: None,
                }],
                runways: vec![FaaRunway {
                    site_number: "00001.".into(),
                    runway_id: "09/27".into(),
                    length_ft: Some(10_000.0),
                    width_ft: Some(150.0),
                    surface: Some("CONC".into()),
                    condition: Some("GOOD".into()),
                    pcn: Some(50.0),
                    pavement_type: Some("R".into()),
                    subgrade: Some("B".into()),
                    tire_pressure: Some("W".into()),
                    determination_method: Some("T".into()),
                    lighting: Some("HIGH".into()),
                    single_wheel_thousand_lb: None,
                    dual_wheel_thousand_lb: Some(100.0),
                    dual_tandem_thousand_lb: None,
                    double_dual_tandem_thousand_lb: None,
                }],
                ends: vec![],
            },
        );
        assert_eq!(airports.len(), 2);
        assert_eq!(airports[0].name, "Authoritative Name");
        assert_eq!(airports[0].runways.len(), 1);
        assert!((airports[0].runways[0].length_m.unwrap() - 3_048.0).abs() < 0.01);
        assert!(
            (airports[0].runways[0]
                .gross_weight_limits
                .dual_wheel_kg
                .unwrap()
                - 45_359.237)
                .abs()
                < 0.01
        );
        assert!(airports[0].source_ids.contains(&"faa_nasr".into()));
    }

    #[test]
    fn extracts_links_from_faa_sections() {
        let html = r#"<h2>Current</h2><a href="/NASR_Subscription/2026-07-09/">Current</a><h2>Archives</h2>"#;
        assert_eq!(extract_hrefs(html), vec!["/NASR_Subscription/2026-07-09/"]);
    }
}
