//! Versioned, provenance-aware catalog records. Raw source files are imported separately.

pub mod space;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source_url: String,
    pub retrieved_on: String,
    pub license: String,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Official,
    CorroboratedPublic,
    Estimate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub domain: String,
    pub max_speed_mps: f64,
    pub endurance_seconds: u64,
    pub sensor_ids: Vec<String>,
    pub communication_ids: Vec<String>,
    pub provenance: Provenance,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("catalog id must contain a namespace and name")]
    InvalidId,
    #[error("max speed must be non-negative")]
    NegativeSpeed,
    #[error("endurance must be non-zero")]
    ZeroEndurance,
    #[error("source URL must use HTTPS")]
    InsecureSource,
}

impl PlatformCatalogEntry {
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.id.split('.').count() < 2 {
            return Err(CatalogError::InvalidId);
        }
        if self.max_speed_mps < 0.0 {
            return Err(CatalogError::NegativeSpeed);
        }
        if self.endurance_seconds == 0 {
            return Err(CatalogError::ZeroEndurance);
        }
        if !self.provenance.source_url.starts_with("https://") {
            return Err(CatalogError::InsecureSource);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unprovenanced_platforms() {
        let entry = PlatformCatalogEntry {
            id: "f35".into(),
            display_name: "F-35A".into(),
            domain: "air".into(),
            max_speed_mps: 1.0,
            endurance_seconds: 1,
            sensor_ids: vec![],
            communication_ids: vec![],
            provenance: Provenance {
                source_url: "http://example.test".into(),
                retrieved_on: "2026-07-10".into(),
                license: "test".into(),
                confidence: Confidence::Estimate,
            },
        };
        assert_eq!(entry.validate(), Err(CatalogError::InvalidId));
    }
}
