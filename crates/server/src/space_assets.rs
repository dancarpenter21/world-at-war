use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;
use serde_json::Value;
use sim_catalog::space::{
    classify_regime, AuthorityConfidence, SatelliteAuthorityAssignment, SatelliteAuthorityKind,
    SourceReference, SpaceAssetIndex, SpaceAssetIndexEntry, SpaceCardManifest,
};

use crate::space_catalog::SpaceCatalogSnapshot;

const DEFAULT_CARDS_DIR: &str = "data/generated/space-cards";
const DEFAULT_SOURCES_PATH: &str = "data/space-cards/sources.json";

#[derive(Clone)]
pub struct SpaceAssetService {
    inner: Arc<Inner>,
}

struct Inner {
    directory: PathBuf,
    index: Option<SpaceAssetIndex>,
    manifest: Option<SpaceCardManifest>,
    sources: BTreeMap<String, SourceReference>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FacetValue {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceAssetsResponse {
    pub catalog_checksum: String,
    pub manifest_version: Option<String>,
    pub enrichment_available: bool,
    pub records: Vec<SpaceAssetIndexEntry>,
    pub facets: BTreeMap<String, Vec<FacetValue>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceAssetDetail {
    pub catalog_checksum: String,
    pub manifest_version: Option<String>,
    pub enrichment_available: bool,
    pub record: SpaceAssetIndexEntry,
    pub raw_orbital_fields: Value,
    pub markdown: String,
    pub sources: Vec<SourceReference>,
    pub authority: SatelliteAuthorityAssignment,
    pub confidence: AuthorityConfidence,
}

impl SpaceAssetService {
    pub async fn load() -> Self {
        let directory = std::env::var("SPACE_CARDS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_CARDS_DIR));
        let index = read_json(directory.join("index.json")).await;
        let manifest = read_json(directory.join("manifest.json")).await;
        let sources: Vec<SourceReference> =
            read_json(DEFAULT_SOURCES_PATH).await.unwrap_or_default();
        Self {
            inner: Arc::new(Inner {
                directory,
                index,
                manifest,
                sources: sources
                    .into_iter()
                    .map(|source| (source.id.clone(), source))
                    .collect(),
            }),
        }
    }

    pub fn list(&self, snapshot: &SpaceCatalogSnapshot) -> SpaceAssetsResponse {
        let matching = self.matching_index(&snapshot.checksum);
        let records = matching
            .map(|index| index.records.clone())
            .unwrap_or_else(|| {
                snapshot
                    .objects
                    .iter()
                    .filter_map(baseline_record)
                    .collect()
            });
        let facets = build_facets(&records);
        SpaceAssetsResponse {
            catalog_checksum: snapshot.checksum.clone(),
            manifest_version: matching.map(|index| index.generator_version.clone()),
            enrichment_available: matching.is_some(),
            records,
            facets,
        }
    }

    pub async fn detail(
        &self,
        snapshot: &SpaceCatalogSnapshot,
        norad: u64,
    ) -> Option<SpaceAssetDetail> {
        let raw = snapshot
            .objects
            .iter()
            .find(|value| integer(value, "NORAD_CAT_ID") == Some(norad))?
            .clone();
        let matching = self.matching_index(&snapshot.checksum);
        let record = matching
            .and_then(|index| {
                index
                    .records
                    .binary_search_by_key(&norad, |record| record.norad_catalog_id)
                    .ok()
                    .map(|position| index.records[position].clone())
            })
            .or_else(|| baseline_record(&raw))?;
        let markdown_path = self
            .inner
            .directory
            .join("cards")
            .join(format!("{norad}.md"));
        let markdown = if matching.is_some() {
            tokio::fs::read_to_string(markdown_path)
                .await
                .ok()
                .map(|value| sanitize_markdown(&value))
        } else {
            None
        };
        let enrichment_available = markdown.is_some();
        let markdown = markdown.unwrap_or_else(|| baseline_markdown(&record));
        let sources = record
            .authority
            .public_source_ids
            .iter()
            .filter_map(|id| self.inner.sources.get(id).cloned())
            .collect();
        Some(SpaceAssetDetail {
            catalog_checksum: snapshot.checksum.clone(),
            manifest_version: matching.map(|index| index.generator_version.clone()),
            enrichment_available,
            authority: record.authority.clone(),
            confidence: record.authority.confidence.clone(),
            record,
            raw_orbital_fields: raw,
            markdown,
            sources,
        })
    }

    fn matching_index(&self, checksum: &str) -> Option<&SpaceAssetIndex> {
        let manifest = self
            .inner
            .manifest
            .as_ref()
            .filter(|manifest| manifest.space_track_checksum == checksum)?;
        self.inner.index.as_ref().filter(|index| {
            index.catalog_checksum == checksum
                && index.generator_version == manifest.generator_version
        })
    }
}

async fn read_json<T: for<'de> serde::Deserialize<'de>>(path: impl AsRef<Path>) -> Option<T> {
    serde_json::from_slice(&tokio::fs::read(path).await.ok()?).ok()
}

fn baseline_record(raw: &Value) -> Option<SpaceAssetIndexEntry> {
    let norad = integer(raw, "NORAD_CAT_ID")?;
    let object_type = string(raw, "OBJECT_TYPE").unwrap_or_else(|| "UNKNOWN".into());
    let nation = string(raw, "COUNTRY_CODE").unwrap_or_else(|| "Unknown".into());
    let us_payload = nation == "US" && object_type == "PAYLOAD";
    Some(SpaceAssetIndexEntry {
        norad_catalog_id: norad,
        cospar_id: string(raw, "OBJECT_ID").filter(|value| value != "UNKNOWN"),
        canonical_name: string(raw, "OBJECT_NAME").unwrap_or_else(|| format!("NORAD {norad}")),
        aliases: Vec::new(),
        nation,
        object_type,
        orbital_regime: classify_regime(
            number(raw, "PERIAPSIS"),
            number(raw, "APOAPSIS"),
            number(raw, "PERIOD"),
            number(raw, "ECCENTRICITY"),
            number(raw, "INCLINATION"),
        )
        .into(),
        operational_status: "Unknown".into(),
        operator: "Unknown".into(),
        mission_category: "Unknown".into(),
        launch_year: string(raw, "LAUNCH_DATE").and_then(|date| date.get(..4)?.parse().ok()),
        radar_size_class: string(raw, "RCS_SIZE").unwrap_or_else(|| "Unknown".into()),
        inclination_deg: number(raw, "INCLINATION"),
        authority: SatelliteAuthorityAssignment {
            authority_id: if us_payload {
                "us.unresolved"
            } else {
                "not_assignable"
            }
            .into(),
            display_name: if us_payload {
                "Unresolved U.S. operator authority"
            } else {
                "Outside U.S. request authority"
            }
            .into(),
            organization: "Unknown".into(),
            kind: SatelliteAuthorityKind::Unresolved,
            game_role_name: None,
            public_source_ids: Vec::new(),
            confidence: AuthorityConfidence::Unresolved,
            allowed_request_types: Vec::new(),
            commandable: false,
        },
        has_enriched_card: false,
    })
}

fn baseline_markdown(record: &SpaceAssetIndexEntry) -> String {
    format!("# {}\n\n## Overview\n\nPinned Space-Track record for NORAD {}. Enrichment is unavailable for this catalog checksum.\n\n## Command Authority\n\n{}. This object is not commandable without a reviewed assignment.\n", safe_text(&record.canonical_name), record.norad_catalog_id, safe_text(&record.authority.display_name))
}

pub fn sanitize_markdown(markdown: &str) -> String {
    markdown
        .lines()
        .filter(|line| !line.trim_start().starts_with('<'))
        .map(|line| {
            if line.to_ascii_lowercase().contains("javascript:") || line.contains("](http://") {
                line.replace("javascript:", "")
                    .replace("](http://", "](https://invalid/")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn safe_text(value: &str) -> String {
    value.replace('<', "&lt;").replace('>', "&gt;")
}

fn build_facets(records: &[SpaceAssetIndexEntry]) -> BTreeMap<String, Vec<FacetValue>> {
    let mut values: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    for record in records {
        for (facet, value) in [
            ("nation", record.nation.clone()),
            ("object_type", record.object_type.clone()),
            ("orbital_regime", record.orbital_regime.clone()),
            ("operational_status", record.operational_status.clone()),
            ("operator", record.operator.clone()),
            ("mission_category", record.mission_category.clone()),
            ("radar_size_class", record.radar_size_class.clone()),
            ("authority_kind", enum_name(&record.authority.kind)),
            (
                "commandability",
                if record.authority.commandable {
                    "commandable".into()
                } else {
                    "not_commandable".into()
                },
            ),
            (
                "launch_year",
                record
                    .launch_year
                    .map_or_else(|| "Unknown".into(), |year| year.to_string()),
            ),
            (
                "inclination_band",
                inclination_band(record.inclination_deg).into(),
            ),
        ] {
            *values.entry(facet).or_default().entry(value).or_default() += 1;
        }
    }
    values
        .into_iter()
        .map(|(facet, counts)| {
            (
                facet.into(),
                counts
                    .into_iter()
                    .map(|(value, count)| FacetValue { value, count })
                    .collect(),
            )
        })
        .collect()
}

fn inclination_band(value: Option<f64>) -> &'static str {
    match value {
        None => "Unknown",
        Some(value) if value < 10.0 => "0–10°",
        Some(value) if value < 45.0 => "10–45°",
        Some(value) if value < 70.0 => "45–70°",
        Some(value) if value < 100.0 => "70–100°",
        Some(_) => "100–180°",
    }
}

fn enum_name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

fn string(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}
fn number(value: &Value, field: &str) -> Option<f64> {
    value
        .get(field)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
}
fn integer(value: &Value, field: &str) -> Option<u64> {
    value
        .get(field)
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizer_removes_html_and_unsafe_links() {
        let value = sanitize_markdown("# Good\n<script>alert(1)</script>\n[x](javascript:bad)");
        assert!(!value.contains("<script>"));
        assert!(!value.contains("javascript:"));
    }

    #[test]
    fn fallback_is_uncommandable() {
        let raw = serde_json::json!({"NORAD_CAT_ID":"5","OBJECT_NAME":"VANGUARD 1","OBJECT_TYPE":"PAYLOAD","COUNTRY_CODE":"US"});
        assert!(!baseline_record(&raw).unwrap().authority.commandable);
    }
}
