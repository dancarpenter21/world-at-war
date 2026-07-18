use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sim_catalog::space::{
    classify_regime, AuthorityConfidence, CoverageStatistics, SatelliteAuthorityAssignment,
    SatelliteAuthorityKind, SourceReference, SpaceAssetIndex, SpaceAssetIndexEntry,
    SpaceCardManifest, REGIME_CLASSIFIER_VERSION, SPACE_CARD_GENERATOR_VERSION,
};

const DEFAULT_INPUT: &str = "data/cache/space-track/latest.json";
const DEFAULT_OUTPUT: &str = "data/generated/space-cards";
const DEFAULT_RULES: &str = "data/space-cards/family-rules.json";
const DEFAULT_SOURCES: &str = "data/space-cards/sources.json";
const DEFAULT_OVERRIDES: &str = "data/space-cards/overrides.json";
const DEFAULT_DOWNLOADS: &str = "data/space-cards/downloads.json";
const DEFAULT_SOURCE_CACHE: &str = "data/cache/space-sources";

#[derive(Debug, Clone, Deserialize)]
struct ExternalDownload {
    id: String,
    url: String,
    filename: String,
}

#[derive(Debug, Clone)]
struct SatcatEnrichment {
    owner: String,
    operational_status: String,
}

#[derive(Deserialize)]
struct Snapshot {
    synced_unix: u64,
    checksum: String,
    objects: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct FamilyRule {
    id: String,
    name_contains: Vec<String>,
    nation: Option<String>,
    operator: String,
    mission_category: String,
    #[serde(default)]
    public_description: Option<String>,
    #[serde(default)]
    sensors: Vec<String>,
    authority_id: String,
    authority_display_name: String,
    authority_organization: String,
    authority_kind: SatelliteAuthorityKind,
    game_role_name: Option<String>,
    source_ids: Vec<String>,
    confidence: AuthorityConfidence,
    allowed_request_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ObjectOverride {
    canonical_name: Option<String>,
    aliases: Option<Vec<String>>,
    operator: Option<String>,
    mission_category: Option<String>,
    public_description: Option<String>,
    sensors: Option<Vec<String>>,
    authority: Option<SatelliteAuthorityAssignment>,
    source_ids: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct ReviewReport {
    catalog_checksum: String,
    unresolved_us_payloads: Vec<ReviewObject>,
    identity_conflicts: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ReviewObject {
    norad_catalog_id: u64,
    cospar_id: Option<String>,
    canonical_name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("space-card-enrich [--input PATH] [--output PATH] [--refresh-sources] [--validate-only]");
        return Ok(());
    }
    let input = option(&args, "--input").unwrap_or_else(|| DEFAULT_INPUT.into());
    let output = option(&args, "--output").unwrap_or_else(|| DEFAULT_OUTPUT.into());
    let validate_only = args.iter().any(|arg| arg == "--validate-only");
    let downloads: Vec<ExternalDownload> = read_json(DEFAULT_DOWNLOADS)?;
    if args.iter().any(|arg| arg == "--refresh-sources") {
        refresh_sources(&downloads, Path::new(DEFAULT_SOURCE_CACHE)).await?;
    }
    generate(Path::new(&input), Path::new(&output), validate_only)
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn generate(input: &Path, output: &Path, validate_only: bool) -> anyhow::Result<()> {
    let snapshot_bytes = fs::read(input).with_context(|| format!("read {}", input.display()))?;
    let snapshot: Snapshot = serde_json::from_slice(&snapshot_bytes)
        .with_context(|| format!("parse {}", input.display()))?;
    if snapshot.objects.is_empty() || snapshot.checksum.len() != 64 {
        bail!("input is not a valid pinned Space-Track snapshot");
    }
    let rules: Vec<FamilyRule> = read_json(DEFAULT_RULES)?;
    let source_list: Vec<SourceReference> = read_json(DEFAULT_SOURCES)?;
    let overrides: BTreeMap<String, ObjectOverride> = read_json(DEFAULT_OVERRIDES)?;
    let downloads: Vec<ExternalDownload> = read_json(DEFAULT_DOWNLOADS)?;
    let satcat =
        load_celestrak_satcat(Path::new(DEFAULT_SOURCE_CACHE).join("celestrak-satcat.txt"));
    let sources: BTreeMap<_, _> = source_list
        .into_iter()
        .map(|source| (source.id.clone(), source))
        .collect();
    validate_definitions(&rules, &sources, &overrides)?;

    let generated_at = snapshot.synced_unix.to_string();
    let mut records = Vec::with_capacity(snapshot.objects.len());
    let mut cards = Vec::new();
    let mut conflicts = Vec::new();
    let mut norad_seen = BTreeSet::new();
    let mut cospar_seen: BTreeMap<String, u64> = BTreeMap::new();
    let mut unresolved = Vec::new();
    let mut payloads = 0;
    let mut us_payloads = 0;

    for raw in &snapshot.objects {
        let norad = integer(raw, "NORAD_CAT_ID")
            .with_context(|| "catalog record lacks a numeric NORAD_CAT_ID")?;
        if !norad_seen.insert(norad) {
            conflicts.push(format!("duplicate NORAD catalog ID {norad}"));
            continue;
        }
        let cospar = string(raw, "OBJECT_ID").filter(|id| id != "UNKNOWN" && !id.trim().is_empty());
        if let Some(id) = &cospar {
            if let Some(previous) = cospar_seen.insert(id.clone(), norad) {
                conflicts.push(format!("COSPAR {id} maps to NORAD {previous} and {norad}"));
            }
        }
        let object_type = string(raw, "OBJECT_TYPE").unwrap_or_else(|| "UNKNOWN".into());
        let is_payload = object_type.eq_ignore_ascii_case("PAYLOAD");
        payloads += usize::from(is_payload);
        let nation = string(raw, "COUNTRY_CODE").unwrap_or_else(|| "Unknown".into());
        let is_us_payload = is_payload && nation == "US";
        us_payloads += usize::from(is_us_payload);
        let original_name = string(raw, "OBJECT_NAME").unwrap_or_else(|| format!("NORAD {norad}"));
        let override_value = overrides.get(&norad.to_string());
        let canonical_name = override_value
            .and_then(|value| value.canonical_name.clone())
            .unwrap_or(original_name);
        let family = rules.iter().find(|rule| {
            rule.nation
                .as_ref()
                .is_none_or(|expected| expected == &nation)
                && rule.name_contains.iter().any(|token| {
                    canonical_name
                        .to_uppercase()
                        .contains(&token.to_uppercase())
                })
        });
        let authority = override_value
            .and_then(|value| value.authority.clone())
            .or_else(|| family.map(authority_from_rule))
            .unwrap_or_else(|| unresolved_authority(is_us_payload));
        if is_us_payload && authority.kind == SatelliteAuthorityKind::Unresolved {
            unresolved.push(ReviewObject {
                norad_catalog_id: norad,
                cospar_id: cospar.clone(),
                canonical_name: canonical_name.clone(),
            });
        }
        let operator = override_value
            .and_then(|value| value.operator.clone())
            .or_else(|| family.map(|value| value.operator.clone()))
            .unwrap_or_else(|| "Unknown".into());
        let satcat_value = satcat.get(&norad);
        let mission_category = override_value
            .and_then(|value| value.mission_category.clone())
            .or_else(|| family.map(|value| value.mission_category.clone()))
            .unwrap_or_else(|| "Unknown".into());
        let public_description = override_value
            .and_then(|value| value.public_description.clone())
            .or_else(|| family.and_then(|value| value.public_description.clone()));
        let sensors = override_value
            .and_then(|value| value.sensors.clone())
            .or_else(|| family.map(|value| value.sensors.clone()))
            .unwrap_or_default();
        let aliases = override_value
            .and_then(|value| value.aliases.clone())
            .unwrap_or_default();
        let regime = classify_regime(
            number(raw, "PERIAPSIS"),
            number(raw, "APOAPSIS"),
            number(raw, "PERIOD"),
            number(raw, "ECCENTRICITY"),
            number(raw, "INCLINATION"),
        )
        .to_owned();
        let mut record = SpaceAssetIndexEntry {
            norad_catalog_id: norad,
            cospar_id: cospar.clone(),
            canonical_name: canonical_name.clone(),
            aliases,
            nation,
            object_type: object_type.clone(),
            orbital_regime: regime,
            operational_status: satcat_value
                .map(|value| value.operational_status.clone())
                .unwrap_or_else(|| "Unknown".into()),
            operator,
            mission_category,
            public_description,
            sensors,
            public_source_ids: Vec::new(),
            launch_year: string(raw, "LAUNCH_DATE")
                .and_then(|date| date.get(..4).and_then(|year| year.parse().ok())),
            radar_size_class: string(raw, "RCS_SIZE").unwrap_or_else(|| "Unknown".into()),
            inclination_deg: number(raw, "INCLINATION"),
            authority,
            has_enriched_card: is_payload,
        };
        if is_payload {
            let mut source_ids = family
                .map(|rule| rule.source_ids.clone())
                .unwrap_or_default();
            source_ids.push("space-track-gp".into());
            source_ids.push("esa-orbits".into());
            if satcat_value.is_some() {
                source_ids.push("celestrak-satcat".into());
            }
            if let Some(ids) = override_value.and_then(|value| value.source_ids.clone()) {
                source_ids.extend(ids);
            }
            source_ids.sort();
            source_ids.dedup();
            record.public_source_ids = source_ids.clone();
            cards.push((
                norad,
                render_card(
                    &record,
                    raw,
                    satcat_value,
                    &source_ids,
                    &sources,
                    &generated_at,
                )?,
            ));
        }
        records.push(record);
    }

    let coverage = CoverageStatistics {
        total_objects: records.len(),
        payloads,
        cards: cards.len(),
        us_payloads,
        us_authority_assignments: records
            .iter()
            .filter(|record| record.object_type == "PAYLOAD" && record.nation == "US")
            .count(),
        unresolved_us_payloads: unresolved.len(),
    };
    if coverage.cards != coverage.payloads
        || coverage.us_authority_assignments != coverage.us_payloads
    {
        bail!("coverage invariant failed");
    }
    if records.iter().any(|record| {
        record.authority.kind == SatelliteAuthorityKind::Unresolved && record.authority.commandable
    }) {
        bail!("unresolved authority assignment cannot be commandable");
    }
    if validate_only {
        println!(
            "validated {} objects, {} payload cards, {} U.S. authority assignments",
            coverage.total_objects, coverage.cards, coverage.us_authority_assignments
        );
        return Ok(());
    }

    let cards_dir = output.join("cards");
    fs::create_dir_all(&cards_dir)?;
    for (norad, markdown) in cards {
        fs::write(cards_dir.join(format!("{norad}.md")), markdown)?;
    }
    records.sort_by_key(|record| record.norad_catalog_id);
    let index = SpaceAssetIndex {
        generator_version: SPACE_CARD_GENERATOR_VERSION.into(),
        regime_classifier_version: REGIME_CLASSIFIER_VERSION.into(),
        catalog_checksum: snapshot.checksum.clone(),
        generated_at: generated_at.clone(),
        records,
    };
    let manifest = SpaceCardManifest {
        generator_version: SPACE_CARD_GENERATOR_VERSION.into(),
        regime_classifier_version: REGIME_CLASSIFIER_VERSION.into(),
        generated_at,
        space_track_checksum: snapshot.checksum.clone(),
        source_versions: sources
            .keys()
            .map(|id| format!("{id}:definition-v1"))
            .chain(source_versions(&downloads, Path::new(DEFAULT_SOURCE_CACHE)))
            .collect(),
        licenses: sources
            .values()
            .map(|source| source.license.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        coverage,
    };
    write_json(output.join("index.json"), &index)?;
    write_json(output.join("manifest.json"), &manifest)?;
    write_json(
        output.join("review-report.json"),
        &ReviewReport {
            catalog_checksum: snapshot.checksum,
            unresolved_us_payloads: unresolved,
            identity_conflicts: conflicts,
        },
    )?;
    println!(
        "generated {} cards at {}",
        manifest.coverage.cards,
        output.display()
    );
    Ok(())
}

async fn refresh_sources(downloads: &[ExternalDownload], cache: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(cache)?;
    let client = reqwest::Client::builder()
        .user_agent("world-at-war-space-card-enrich/1.0")
        .build()?;
    for download in downloads {
        if !download.url.starts_with("https://") {
            bail!("external source {} must use HTTPS", download.id);
        }
        let response = client
            .get(&download.url)
            .send()
            .await
            .with_context(|| format!("download {}", download.id))?;
        if !response.status().is_success() {
            bail!(
                "external source {} returned HTTP {}",
                download.id,
                response.status()
            );
        }
        let bytes = response.bytes().await?;
        if bytes.is_empty() {
            bail!("external source {} returned no data", download.id);
        }
        let path = cache.join(&download.filename);
        let temporary = path.with_extension("download");
        fs::write(&temporary, &bytes)?;
        fs::rename(temporary, path)?;
    }
    Ok(())
}

fn load_celestrak_satcat(path: PathBuf) -> BTreeMap<u64, SatcatEnrichment> {
    let Ok(contents) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    contents.lines().filter_map(parse_celestrak_line).collect()
}

fn parse_celestrak_line(line: &str) -> Option<(u64, SatcatEnrichment)> {
    if line.len() < 54 || !line.is_char_boundary(13) || !line.is_char_boundary(54) {
        return None;
    }
    let norad = line.get(13..18)?.trim().parse().ok()?;
    let status = line
        .as_bytes()
        .get(21)
        .copied()
        .map(char::from)
        .unwrap_or(' ');
    let operational_status = match status {
        '+' => "operational",
        '-' => "nonoperational",
        'P' => "partially_operational",
        'B' => "backup",
        'S' => "spare",
        'X' => "extended_mission",
        'D' => "decayed",
        _ => "Unknown",
    };
    Some((
        norad,
        SatcatEnrichment {
            owner: line.get(49..54)?.trim().to_owned(),
            operational_status: operational_status.into(),
        },
    ))
}

fn source_versions(downloads: &[ExternalDownload], cache: &Path) -> Vec<String> {
    downloads
        .iter()
        .map(|download| {
            fs::read(cache.join(&download.filename))
                .ok()
                .map(|bytes| format!("{}:{}", download.id, hex_digest(&bytes)))
                .unwrap_or_else(|| format!("{}:not_cached", download.id))
        })
        .collect()
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &str) -> anyhow::Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("read {path}"))?)
        .with_context(|| format!("parse {path}"))
}

fn write_json(path: PathBuf, value: &impl Serialize) -> anyhow::Result<()> {
    fs::create_dir_all(path.parent().context("output has no parent")?)?;
    fs::write(path, serde_json::to_vec(value)?)?;
    Ok(())
}

fn validate_definitions(
    rules: &[FamilyRule],
    sources: &BTreeMap<String, SourceReference>,
    overrides: &BTreeMap<String, ObjectOverride>,
) -> anyhow::Result<()> {
    for source in sources.values() {
        if !source.url.starts_with("https://") {
            bail!("source {} must use HTTPS", source.id);
        }
    }
    for rule in rules {
        if rule.id.trim().is_empty() || rule.name_contains.is_empty() {
            bail!("family rule IDs and match tokens are required");
        }
        for source in &rule.source_ids {
            if !sources.contains_key(source) {
                bail!("family rule {} references unknown source {source}", rule.id);
            }
        }
    }
    for id in overrides.keys() {
        id.parse::<u64>()
            .with_context(|| format!("override key {id} is not a NORAD ID"))?;
    }
    Ok(())
}

fn authority_from_rule(rule: &FamilyRule) -> SatelliteAuthorityAssignment {
    SatelliteAuthorityAssignment {
        authority_id: rule.authority_id.clone(),
        display_name: rule.authority_display_name.clone(),
        organization: rule.authority_organization.clone(),
        kind: rule.authority_kind.clone(),
        game_role_name: rule.game_role_name.clone(),
        public_source_ids: rule.source_ids.clone(),
        confidence: rule.confidence.clone(),
        allowed_request_types: rule.allowed_request_types.clone(),
        commandable: true,
    }
}

fn unresolved_authority(us_payload: bool) -> SatelliteAuthorityAssignment {
    SatelliteAuthorityAssignment {
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
    }
}

fn render_card(
    record: &SpaceAssetIndexEntry,
    raw: &Value,
    satcat: Option<&SatcatEnrichment>,
    source_ids: &[String],
    sources: &BTreeMap<String, SourceReference>,
    generated_at: &str,
) -> anyhow::Result<String> {
    let source_values: Vec<_> = source_ids.iter().filter_map(|id| sources.get(id)).collect();
    let source_urls: Vec<_> = source_values
        .iter()
        .map(|source| source.url.clone())
        .collect();
    let family_confidence = if record.operator == "Unknown" {
        "unresolved".into()
    } else {
        enum_name(&record.authority.confidence)
    };
    let status_confidence = if satcat.is_some() {
        "official"
    } else {
        "unresolved"
    };
    let field_metadata = format!(
        "field_confidence:\n  canonical_name: official\n  cospar_id: official\n  nation: official\n  operator: {}\n  owner: {}\n  mission_category: {}\n  operational_status: {}\n  launch: official\n  physical_characteristics: unresolved\n  orbital_regime: inferred\n  authority: {}\nfield_provenance:\n  canonical_name: [space-track-gp]\n  cospar_id: [space-track-gp]\n  nation: [space-track-gp]\n  operator: {}\n  owner: {}\n  mission_category: {}\n  operational_status: {}\n  launch: [space-track-gp]\n  orbital_regime: [space-track-gp, esa-orbits]\n  authority: {}",
        family_confidence,
        status_confidence,
        family_confidence,
        status_confidence,
        enum_name(&record.authority.confidence),
        serde_json::to_string(source_ids)?,
        if satcat.is_some() { "[celestrak-satcat]" } else { "[]" },
        serde_json::to_string(source_ids)?,
        if satcat.is_some() { "[celestrak-satcat]" } else { "[]" },
        serde_json::to_string(&record.authority.public_source_ids)?,
    );
    let quote = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"Unknown\"".into());
    let redacted = "[REDACTED]";
    let sensor_markdown = if record.sensors.is_empty() {
        redacted.into()
    } else {
        record
            .sensors
            .iter()
            .map(|sensor| format!("- {}", escape_markdown(sensor)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut markdown = format!(
        "---\nnorad_catalog_id: {}\ncospar_id: {}\ncanonical_name: {}\naliases: {}\nnation: {}\noperator: {}\nowner: {}\nmanufacturer: {}\nmission_category: {}\npublic_description: {}\nsensors: {}\noperational_status: {}\nlaunch_date: {}\nlaunch_site: {}\nbus: {}\nmass: {}\ndimensions: {}\norbital_regime: {}\ngenerated_at: {}\nauthority_assignment_id: {}\nauthority_kind: {}\nauthority_confidence: {}\ncommandable: {}\nsource_urls: {}\n{}\n---\n\n# {}\n\n## Overview\n\n{}\n\nPublic catalog payload {} (COSPAR {}). Current orbital elements are displayed live from the game-pinned Space-Track snapshot.\n\n## Public Mission\n\nMission category: **{}**.\n\n## Publicly Reported Sensors and Payloads\n\n{}\n\n## Operator and Operations\n\nOperator: **{}**. Operational status: **{}**.\n\n## Command Authority\n\n{} — {}. This assignment is {} and is separate from ownership.\n\n## Physical Characteristics\n\nBus, mass, and dimensions: **[REDACTED]** unless a reviewed public override supplies them.\n\n## Registration\n\nNation/state: **{}**. Launch date: **{}**. Launch site: **{}**.\n\n## Sources\n\n",
        record.norad_catalog_id,
        quote(record.cospar_id.as_deref().unwrap_or("Unknown")),
        quote(&record.canonical_name),
        serde_json::to_string(&record.aliases)?,
        quote(&record.nation), quote(&record.operator), quote(satcat.map(|value| value.owner.as_str()).unwrap_or("Unknown")), quote("Unknown"),
        quote(&record.mission_category),
        quote(record.public_description.as_deref().unwrap_or(redacted)),
        serde_json::to_string(&record.sensors)?,
        quote(&record.operational_status),
        quote(string(raw, "LAUNCH_DATE").as_deref().unwrap_or("Unknown")),
        quote(string(raw, "SITE").as_deref().unwrap_or("Unknown")),
        quote("Unknown"), quote("Unknown"), quote("Unknown"), quote(&record.orbital_regime),
        quote(generated_at), quote(&record.authority.authority_id),
        quote(&enum_name(&record.authority.kind)),
        quote(&enum_name(&record.authority.confidence)),
        record.authority.commandable, serde_json::to_string(&source_urls)?, field_metadata,
        escape_markdown(&record.canonical_name),
        escape_markdown(record.public_description.as_deref().unwrap_or(redacted)),
        record.norad_catalog_id,
        escape_markdown(record.cospar_id.as_deref().unwrap_or("Unknown")),
        escape_markdown(if record.mission_category == "Unknown" { redacted } else { &record.mission_category }),
        sensor_markdown,
        escape_markdown(&record.operator),
        escape_markdown(&record.operational_status), escape_markdown(&record.authority.display_name),
        escape_markdown(&record.authority.organization),
        escape_markdown(&enum_name(&record.authority.confidence)),
        escape_markdown(&record.nation), escape_markdown(string(raw, "LAUNCH_DATE").as_deref().unwrap_or("Unknown")),
        escape_markdown(string(raw, "SITE").as_deref().unwrap_or("Unknown")),
    );
    if source_values.is_empty() {
        markdown
            .push_str("- Space-Track GP pinned game snapshot (orbital fields and identifiers).\n");
    } else {
        for source in source_values {
            markdown.push_str(&format!(
                "- [{}]({}) — {}, retrieved {}.\n",
                escape_markdown(&source.title),
                source.url,
                escape_markdown(&source.license),
                escape_markdown(&source.retrieved_on)
            ));
        }
    }
    if markdown.contains('<') || markdown.contains("javascript:") || markdown.contains("http://") {
        bail!(
            "unsafe Markdown generated for NORAD {}",
            record.norad_catalog_id
        );
    }
    Ok(markdown)
}

fn escape_markdown(value: &str) -> String {
    let value = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '[' | ']' | '*' | '_' | '`') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
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
    fn markdown_escapes_raw_html() {
        assert_eq!(
            escape_markdown("<script>*x*</script>"),
            "&lt;script&gt;\\*x\\*&lt;/script&gt;"
        );
    }

    #[test]
    fn unresolved_assignments_are_never_commandable() {
        assert!(!unresolved_authority(true).commandable);
        assert!(!unresolved_authority(false).commandable);
    }

    #[test]
    fn filenames_are_numeric_norad_ids() {
        let path = Path::new("cards").join(format!("{}.md", 25544_u64));
        assert_eq!(path.to_string_lossy(), "cards/25544.md");
    }

    #[test]
    fn checksum_is_stable_for_same_bytes() {
        let one = Sha256::digest(b"snapshot");
        let two = Sha256::digest(b"snapshot");
        assert_eq!(one[..], two[..]);
    }

    #[test]
    fn celestrak_join_uses_exact_norad_column() {
        let line = "1957-001A    00001   D SL-1 R/B                  CIS    1957-10-04";
        let (norad, value) = parse_celestrak_line(line).unwrap();
        assert_eq!(norad, 1);
        assert_eq!(value.owner, "CIS");
        assert_eq!(value.operational_status, "decayed");
    }
}
