//! Public-source fuel facts and explicit gaps. Facts alone never enable a flight model.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub field: String,
    pub confidence: super::Confidence,
    pub value: f64,
    pub unit: String,
    pub source_url: String,
    pub locator: String,
    pub applicability: String,
    pub retrieved_on: String,
    pub license: String,
    pub normalized_value: Option<f64>,
    pub normalized_unit: Option<String>,
    pub conversion: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aircraft {
    pub id: String,
    pub display_name: String,
    pub configuration: String,
    pub facts: Vec<Fact>,
    pub missing: Vec<String>,
    pub aerial_refueling: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub version: u32,
    pub aircraft: Vec<Aircraft>,
}
impl Catalog {
    pub fn bundled() -> Result<Self, String> {
        let catalog: Self = serde_json::from_str(include_str!(
            "../../../data/logistics/aircraft-fuel.v1.json"
        ))
        .map_err(|e| e.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = std::collections::BTreeSet::new();
        if self.version != 1 {
            return Err("unsupported aircraft fuel catalog".into());
        }
        for a in &self.aircraft {
            if !ids.insert(&a.id)
                || a.id.is_empty()
                || a.configuration.is_empty()
                || a.missing.is_empty()
            {
                return Err(
                    "aircraft records require unique IDs, configuration and explicit coverage gaps"
                        .into(),
                );
            }
            for f in &a.facts {
                if !f.value.is_finite()
                    || f.value <= 0.0
                    || !f.source_url.starts_with("https://")
                    || [
                        &f.field,
                        &f.unit,
                        &f.locator,
                        &f.applicability,
                        &f.retrieved_on,
                        &f.license,
                    ]
                    .iter()
                    .any(|s| s.trim().is_empty())
                {
                    return Err("invalid or unsourced aircraft fuel fact".into());
                }
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_covers_existing_blue_types_without_claiming_missing_tables() {
        let c = Catalog::bundled().unwrap();
        assert_eq!(c.aircraft.len(), 11);
        for a in c.aircraft {
            assert!(a.missing.iter().any(|s| s.contains("performance")));
        }
    }
    #[test]
    fn kc46_distinguishes_capacity_boom_drogue_and_receiver_rates() {
        let c = Catalog::bundled().unwrap();
        let k = c.aircraft.iter().find(|a| a.id == "kc-46a").unwrap();
        assert!(k
            .facts
            .iter()
            .any(|f| f.field == "fuel_capacity" && f.value == 96297.0 && f.unit == "kg"));
        assert!(k
            .facts
            .iter()
            .any(|f| f.field == "boom_offload_rate" && f.value == 1200.0));
        assert!(k
            .facts
            .iter()
            .any(|f| f.field == "drogue_offload_rate" && f.value == 400.0));
    }
}
