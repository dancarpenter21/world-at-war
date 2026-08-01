//! Versioned communications catalogs and C2 message primitives.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use c3mesh::{FrequencyBand, QueueConfig, QueueDiscipline};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Official,
    Manufacturer,
    CorroboratedPublic,
    Estimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceReference {
    pub id: String,
    pub title: String,
    pub url: String,
    pub retrieved_on: String,
    pub license: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioModeProfile {
    pub id: String,
    pub band: FrequencyBand,
    pub nominal_bit_rate_bps: u64,
    pub max_range_m: f64,
    pub shared_medium: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfile {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    #[serde(default)]
    pub radio_modes: Vec<RadioModeProfile>,
    pub queue_capacity_packets: usize,
    pub queue_capacity_bytes: usize,
    pub processing_delay_ns: u64,
    pub confidence: Confidence,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub estimate_rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformProfile {
    pub id: String,
    pub display_name: String,
    pub device_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformAssignment {
    pub entity_name: String,
    pub platform_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageProfile {
    pub id: String,
    pub family: String,
    pub display_name: String,
    pub default_priority: u8,
    pub ttl_hops: u16,
    pub expiry_ticks: u64,
    pub reliable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicyProfile {
    pub id: String,
    pub display_name: String,
    pub queue_discipline: QueueDiscipline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommunicationsCatalog {
    pub version: u32,
    pub as_of: String,
    pub sources: Vec<SourceReference>,
    pub devices: Vec<DeviceProfile>,
    pub platforms: Vec<PlatformProfile>,
    pub assignments: Vec<PlatformAssignment>,
    pub messages: Vec<MessageProfile>,
    pub policies: Vec<NetworkPolicyProfile>,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("read communications catalog: {0}")]
    Read(#[from] std::io::Error),
    #[error("parse communications catalog: {0}")]
    Parse(String),
    #[error("invalid communications catalog: {0}")]
    Invalid(String),
}

impl CommunicationsCatalog {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let bytes = fs::read(path)?;
        let catalog: Self = yaml_serde::from_slice(&bytes)
            .map_err(|error| CatalogError::Parse(error.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.version == 0 || self.as_of.trim().is_empty() {
            return Err(CatalogError::Invalid(
                "version and as_of are required".into(),
            ));
        }
        let sources: BTreeSet<_> = self.sources.iter().map(|value| value.id.as_str()).collect();
        if sources.len() != self.sources.len()
            || self
                .sources
                .iter()
                .any(|value| value.id.trim().is_empty() || !value.url.starts_with("https://"))
        {
            return Err(CatalogError::Invalid(
                "sources must be unique HTTPS references".into(),
            ));
        }
        let devices: BTreeMap<_, _> = self
            .devices
            .iter()
            .map(|value| (value.id.as_str(), value))
            .collect();
        if devices.len() != self.devices.len() {
            return Err(CatalogError::Invalid("duplicate device profile".into()));
        }
        for device in &self.devices {
            if device.id.trim().is_empty() || device.display_name.trim().is_empty() {
                return Err(CatalogError::Invalid(
                    "device ids and display names are required".into(),
                ));
            }
            if device.queue_capacity_packets == 0 || device.queue_capacity_bytes == 0 {
                return Err(CatalogError::Invalid(format!(
                    "{} has an empty queue",
                    device.id
                )));
            }
            let mode_ids: BTreeSet<_> = device
                .radio_modes
                .iter()
                .map(|mode| mode.id.as_str())
                .collect();
            if mode_ids.len() != device.radio_modes.len() {
                return Err(CatalogError::Invalid(format!(
                    "{} has duplicate radio modes",
                    device.id
                )));
            }
            if device.confidence == Confidence::Estimate
                && device
                    .estimate_rationale
                    .as_deref()
                    .is_none_or(str::is_empty)
            {
                return Err(CatalogError::Invalid(format!(
                    "{} needs an estimate rationale",
                    device.id
                )));
            }
            if device
                .source_ids
                .iter()
                .any(|id| !sources.contains(id.as_str()))
            {
                return Err(CatalogError::Invalid(format!(
                    "{} references an unknown source",
                    device.id
                )));
            }
            for mode in &device.radio_modes {
                if mode.band.lower_hz >= mode.band.upper_hz
                    || mode.nominal_bit_rate_bps == 0
                    || !mode.max_range_m.is_finite()
                    || mode.max_range_m <= 0.0
                {
                    return Err(CatalogError::Invalid(format!(
                        "{} has an invalid radio mode",
                        device.id
                    )));
                }
            }
        }
        let platforms: BTreeMap<_, _> = self
            .platforms
            .iter()
            .map(|value| (value.id.as_str(), value))
            .collect();
        if platforms.len() != self.platforms.len()
            || self.platforms.iter().any(|profile| {
                profile.device_ids.is_empty()
                    || profile
                        .device_ids
                        .iter()
                        .any(|id| !devices.contains_key(id.as_str()))
            })
        {
            return Err(CatalogError::Invalid(
                "platform profiles must resolve devices".into(),
            ));
        }
        if self
            .assignments
            .iter()
            .any(|assignment| !platforms.contains_key(assignment.platform_profile_id.as_str()))
        {
            return Err(CatalogError::Invalid(
                "assignment references an unknown platform".into(),
            ));
        }
        let assigned_entities: BTreeSet<_> = self
            .assignments
            .iter()
            .map(|assignment| assignment.entity_name.as_str())
            .collect();
        if assigned_entities.len() != self.assignments.len()
            || self
                .assignments
                .iter()
                .any(|assignment| assignment.entity_name.trim().is_empty())
        {
            return Err(CatalogError::Invalid(
                "entity assignments must be named and unique".into(),
            ));
        }
        let message_ids: BTreeSet<_> = self
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect();
        let policy_ids: BTreeSet<_> = self
            .policies
            .iter()
            .map(|policy| policy.id.as_str())
            .collect();
        if self.messages.is_empty()
            || message_ids.len() != self.messages.len()
            || self.messages.iter().any(|message| {
                message.id.trim().is_empty()
                    || message.family.trim().is_empty()
                    || message.ttl_hops == 0
                    || message.expiry_ticks == 0
            })
        {
            return Err(CatalogError::Invalid(
                "message profiles must be unique and have valid TTL/expiry values".into(),
            ));
        }
        if self.policies.is_empty()
            || policy_ids.len() != self.policies.len()
            || self
                .policies
                .iter()
                .any(|policy| policy.id.trim().is_empty())
        {
            return Err(CatalogError::Invalid(
                "network policies must be named and unique".into(),
            ));
        }
        Ok(())
    }

    pub fn checksum(&self) -> String {
        let normalized = serde_json::to_vec(self).expect("catalog serialization is infallible");
        format!("{:x}", Sha256::digest(normalized))
    }

    /// Returns a checksum for the public-safe message definition subset only.
    /// This is pinned separately from equipment facts so externally supplied
    /// definition packs can be compared without changing topology identity.
    pub fn message_pack_checksum(&self) -> String {
        let normalized =
            serde_json::to_vec(&self.messages).expect("message serialization is infallible");
        format!("{:x}", Sha256::digest(normalized))
    }

    pub fn assignment_for(&self, entity_name: &str) -> Option<&PlatformProfile> {
        let profile = self
            .assignments
            .iter()
            .find(|value| value.entity_name == entity_name)?;
        self.platforms
            .iter()
            .find(|value| value.id == profile.platform_profile_id)
    }

    pub fn queue_for(&self, device_id: &str, policy_id: &str) -> Option<QueueConfig> {
        let device = self.devices.iter().find(|value| value.id == device_id)?;
        let policy = self.policies.iter().find(|value| value.id == policy_id)?;
        Some(QueueConfig {
            max_packets: Some(device.queue_capacity_packets),
            max_bytes: Some(device.queue_capacity_bytes),
            discipline: policy.queue_discipline,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageHeader {
    pub origin_role_id: Uuid,
    pub origin_entity_id: Uuid,
    pub recipient_entity_id: Uuid,
    pub classification: String,
    pub priority: u8,
    pub created_tick: u64,
    pub expires_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct C2Message {
    pub id: Uuid,
    pub profile_id: String,
    pub header: MessageHeader,
    pub fields: BTreeMap<String, serde_json::Value>,
    pub rendered_text: String,
}

impl C2Message {
    pub fn encoded(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("message serialization is infallible")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    Queued,
    InTransit,
    Delivered,
    Acknowledged,
    Retrying,
    Dropped,
    Expired,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_catalog_is_valid_and_stable() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/communications/catalog.yaml");
        let catalog = CommunicationsCatalog::load(path).unwrap();
        assert_eq!(catalog.assignments.len(), 66);
        assert_eq!(catalog.checksum().len(), 64);
        assert_eq!(catalog.message_pack_checksum().len(), 64);
    }

    #[test]
    fn rejects_duplicate_entity_assignments() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/communications/catalog.yaml");
        let mut catalog = CommunicationsCatalog::load(path).unwrap();
        catalog.assignments.push(catalog.assignments[0].clone());
        assert!(matches!(catalog.validate(), Err(CatalogError::Invalid(_))));
    }
}
