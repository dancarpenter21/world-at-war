//! Fictional gameplay presets; networking references do not establish equipment capabilities.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub as_of: String,
    pub sources: Vec<Source>,
    pub links: Vec<LinkProfile>,
    pub weapons: Vec<WeaponProfile>,
    pub fits: Vec<PlatformFit>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub confidence: String,
    pub id: String,
    pub url: String,
    pub retrieved_on: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkProfile {
    pub id: String,
    pub band: c3mesh::FrequencyBand,
    pub bit_rate_bps: u64,
    pub range_m: f64,
    pub satellite: bool,
    pub estimate_rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Seeker {
    None,
    Local,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponProfile {
    pub id: String,
    pub name: String,
    pub receiver_links: Vec<String>,
    pub sources: Vec<String>,
    pub retarget: bool,
    pub handoff: bool,
    pub telemetry: bool,
    pub seeker: Seeker,
    pub speed_mps: f64,
    pub lifetime_seconds: f64,
    pub acquisition_m: f64,
    pub effect_radius_m: f64,
    pub update_interval_ticks: u64,
    pub max_observation_age_ticks: u64,
    pub estimate_rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformFit {
    pub platform: String,
    pub weapons: Vec<String>,
    pub transmitter_links: Vec<String>,
    pub evidence: String,
    pub sources: Vec<String>,
}
impl Catalog {
    pub fn bundled() -> Result<Self, String> {
        let catalog: Self =
            serde_json::from_str(include_str!("../../../../data/weapons/catalog.json"))
                .map_err(|e| e.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn weapon(&self, id: &str) -> Option<&WeaponProfile> {
        self.weapons.iter().find(|w| w.id == id)
    }
    pub fn link(&self, id: &str) -> Option<&LinkProfile> {
        self.links.iter().find(|w| w.id == id)
    }
    pub fn validate(&self) -> Result<(), String> {
        let source_ids: BTreeSet<_> = self.sources.iter().map(|s| s.id.as_str()).collect();
        let links: BTreeSet<_> = self.links.iter().map(|s| s.id.as_str()).collect();
        let weapons: BTreeSet<_> = self.weapons.iter().map(|s| s.id.as_str()).collect();
        let fits: BTreeSet<_> = self.fits.iter().map(|s| s.platform.as_str()).collect();
        if self.version == 0
            || self.as_of.is_empty()
            || source_ids.len() != self.sources.len()
            || links.len() != self.links.len()
            || weapons.len() != self.weapons.len()
            || fits.len() != self.fits.len()
        {
            return Err("duplicate or invalid catalog identity".into());
        }
        for s in &self.sources {
            if !matches!(
                s.confidence.as_str(),
                "official" | "manufacturer" | "estimate"
            ) || s.id.is_empty()
                || !s.url.starts_with("https://")
                || s.retrieved_on.is_empty()
            {
                return Err("invalid capability source".into());
            }
        }
        for l in &self.links {
            if l.id.is_empty()
                || l.band.lower_hz >= l.band.upper_hz
                || l.bit_rate_bps == 0
                || !l.range_m.is_finite()
                || l.range_m <= 0.0
                || l.estimate_rationale.is_empty()
            {
                return Err("invalid link estimate".into());
            }
        }
        for w in &self.weapons {
            if w.id.is_empty()
                || w.sources.iter().any(|s| !source_ids.contains(s.as_str()))
                || w.receiver_links.iter().any(|s| !links.contains(s.as_str()))
                || w.update_interval_ticks == 0
                || w.max_observation_age_ticks == 0
                || [w.speed_mps, w.lifetime_seconds, w.effect_radius_m]
                    .iter()
                    .any(|v| !v.is_finite() || *v <= 0.0)
                || !w.acquisition_m.is_finite()
                || w.acquisition_m < 0.0
                || w.estimate_rationale.is_empty()
            {
                return Err(format!("invalid weapon {}", w.id));
            }
        }
        for f in &self.fits {
            if f.platform.is_empty()
                || f.evidence.is_empty()
                || f.weapons.iter().any(|w| !weapons.contains(w.as_str()))
                || f.transmitter_links
                    .iter()
                    .any(|l| !links.contains(l.as_str()))
                || f.sources.iter().any(|s| !source_ids.contains(s.as_str()))
            {
                return Err("invalid platform fit".into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_capabilities_are_valid() {
        Catalog::bundled().unwrap();
    }
    #[test]
    fn rejects_unknown_receiver_link() {
        let mut c = Catalog::bundled().unwrap();
        c.weapons[0].receiver_links.push("unknown".into());
        assert!(c.validate().is_err());
    }
}
