use serde::{Deserialize, Serialize};

pub const SPACE_CARD_GENERATOR_VERSION: &str = "1.1.0";
pub const REGIME_CLASSIFIER_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SatelliteAuthorityKind {
    MilitaryRole,
    CivilOperator,
    CommercialOperator,
    AcademicOperator,
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityConfidence {
    Official,
    CorroboratedPublic,
    Inferred,
    Unresolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceReference {
    pub id: String,
    pub title: String,
    pub url: String,
    pub retrieved_on: String,
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SatelliteAuthorityAssignment {
    pub authority_id: String,
    pub display_name: String,
    pub organization: String,
    pub kind: SatelliteAuthorityKind,
    pub game_role_name: Option<String>,
    pub public_source_ids: Vec<String>,
    pub confidence: AuthorityConfidence,
    pub allowed_request_types: Vec<String>,
    pub commandable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpaceAssetIndexEntry {
    pub norad_catalog_id: u64,
    pub cospar_id: Option<String>,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub nation: String,
    pub object_type: String,
    pub orbital_regime: String,
    pub operational_status: String,
    pub operator: String,
    pub mission_category: String,
    #[serde(default)]
    pub public_description: Option<String>,
    #[serde(default)]
    pub sensors: Vec<String>,
    #[serde(default)]
    pub public_source_ids: Vec<String>,
    pub launch_year: Option<u16>,
    pub radar_size_class: String,
    pub inclination_deg: Option<f64>,
    pub authority: SatelliteAuthorityAssignment,
    pub has_enriched_card: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpaceAssetIndex {
    pub generator_version: String,
    pub regime_classifier_version: String,
    pub catalog_checksum: String,
    pub generated_at: String,
    pub records: Vec<SpaceAssetIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverageStatistics {
    pub total_objects: usize,
    pub payloads: usize,
    pub cards: usize,
    pub us_payloads: usize,
    pub us_authority_assignments: usize,
    pub unresolved_us_payloads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceCardManifest {
    pub generator_version: String,
    pub regime_classifier_version: String,
    pub generated_at: String,
    pub space_track_checksum: String,
    pub source_versions: Vec<String>,
    pub licenses: Vec<String>,
    pub coverage: CoverageStatistics,
}

pub fn classify_regime(
    periapsis_km: Option<f64>,
    apoapsis_km: Option<f64>,
    period_minutes: Option<f64>,
    eccentricity: Option<f64>,
    inclination_deg: Option<f64>,
) -> &'static str {
    let (Some(periapsis), Some(apoapsis)) = (periapsis_km, apoapsis_km) else {
        return "unknown";
    };
    let eccentricity = eccentricity.unwrap_or(0.0);
    let inclination = inclination_deg.unwrap_or(0.0);
    let period = period_minutes.unwrap_or(0.0);
    if eccentricity >= 0.25 && apoapsis >= 10_000.0 {
        return if inclination >= 50.0 {
            "heo"
        } else {
            "gto_cross_regime"
        };
    }
    if (apoapsis - 35_786.0).abs() <= 2_000.0
        && (periapsis - 35_786.0).abs() <= 2_000.0
        && (period - 1_436.0).abs() <= 90.0
    {
        return if inclination <= 5.0 { "geo" } else { "gso" };
    }
    if apoapsis < 2_000.0 {
        "leo"
    } else if periapsis >= 2_000.0 && apoapsis < 30_000.0 {
        "meo"
    } else if periapsis > 60_000.0 {
        "deep_space_other"
    } else {
        "cross_regime"
    }
}

#[cfg(test)]
mod tests {
    use super::classify_regime;

    #[test]
    fn classifies_public_orbit_regimes() {
        assert_eq!(
            classify_regime(Some(500.0), Some(510.0), Some(95.0), Some(0.0), Some(51.6)),
            "leo"
        );
        assert_eq!(
            classify_regime(
                Some(20_100.0),
                Some(20_300.0),
                Some(718.0),
                Some(0.01),
                Some(55.0)
            ),
            "meo"
        );
        assert_eq!(
            classify_regime(
                Some(35_770.0),
                Some(35_800.0),
                Some(1_436.0),
                Some(0.0),
                Some(0.1)
            ),
            "geo"
        );
        assert_eq!(
            classify_regime(
                Some(35_770.0),
                Some(35_800.0),
                Some(1_436.0),
                Some(0.0),
                Some(12.0)
            ),
            "gso"
        );
        assert_eq!(
            classify_regime(
                Some(250.0),
                Some(35_800.0),
                Some(700.0),
                Some(0.7),
                Some(28.0)
            ),
            "gto_cross_regime"
        );
        assert_eq!(
            classify_regime(
                Some(900.0),
                Some(39_000.0),
                Some(720.0),
                Some(0.7),
                Some(63.4)
            ),
            "heo"
        );
        assert_eq!(
            classify_regime(None, Some(500.0), None, None, None),
            "unknown"
        );
    }
}
