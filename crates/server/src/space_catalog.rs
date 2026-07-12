use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

const CACHE_PATH: &str = "data/cache/space-track/latest.json";
const MAX_AGE_SECONDS: u64 = 86_400;
const MIN_SYNC_INTERVAL_SECONDS: u64 = 3_600;
const GP_URL: &str = "https://www.space-track.org/basicspacedata/query/class/gp/decay_date/null-val/epoch/%3Enow-10/orderby/norad_cat_id/format/json";

#[derive(Clone)]
pub struct SpaceCatalogService {
    inner: Arc<RwLock<Inner>>,
    client: reqwest::Client,
}

struct Inner {
    credentials: Option<Credentials>,
    snapshots: BTreeMap<String, SpaceCatalogSnapshot>,
    latest_checksum: Option<String>,
    syncing: bool,
    last_successful_sync_unix: Option<u64>,
    error: Option<String>,
}

struct Credentials {
    username: String,
    password: SecretString,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceCatalogSnapshot {
    pub synced_unix: u64,
    pub source: String,
    pub checksum: String,
    pub objects: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpaceCatalogStatus {
    pub setup_auth_required: bool,
    pub remembered_credentials: bool,
    pub configured: bool,
    pub syncing: bool,
    pub usable: bool,
    pub stale: bool,
    pub synced_unix: Option<u64>,
    pub age_seconds: Option<u64>,
    pub object_count: usize,
    pub checksum: Option<String>,
    pub error: Option<String>,
}

impl SpaceCatalogService {
    pub async fn load() -> anyhow::Result<Self> {
        let snapshot = match tokio::fs::read(CACHE_PATH).await {
            Ok(bytes) => serde_json::from_slice(&bytes).ok(),
            Err(_) => None,
        };
        let credentials = match (
            std::env::var("SPACETRACK_USERNAME").ok(),
            std::env::var("SPACETRACK_PASSWORD").ok(),
        ) {
            (Some(username), Some(password)) if !username.is_empty() && !password.is_empty() => {
                Some(Credentials {
                    username,
                    password: SecretString::from(password),
                })
            }
            _ => None,
        };
        let latest_checksum = snapshot
            .as_ref()
            .map(|snapshot: &SpaceCatalogSnapshot| snapshot.checksum.clone());
        let last_successful_sync_unix = snapshot.as_ref().map(|snapshot| snapshot.synced_unix);
        let snapshots = snapshot
            .into_iter()
            .map(|snapshot| (snapshot.checksum.clone(), snapshot))
            .collect();
        Ok(Self {
            inner: Arc::new(RwLock::new(Inner {
                credentials,
                snapshots,
                latest_checksum,
                syncing: false,
                last_successful_sync_unix,
                error: None,
            })),
            client: reqwest::Client::builder()
                .cookie_store(true)
                .user_agent("world-at-war/0.1")
                .build()?,
        })
    }

    pub async fn configure_and_sync(
        &self,
        username: String,
        password: String,
    ) -> anyhow::Result<SpaceCatalogStatus> {
        self.configure(username, password).await?;
        self.sync(false).await?;
        Ok(self.status().await)
    }

    pub async fn restore_credentials(
        &self,
        username: String,
        password: String,
    ) -> anyhow::Result<SpaceCatalogStatus> {
        self.configure(username, password).await?;
        if !self.status().await.usable {
            self.sync(false).await?;
        }
        Ok(self.status().await)
    }

    async fn configure(&self, username: String, password: String) -> anyhow::Result<()> {
        if username.trim().is_empty() || password.is_empty() {
            anyhow::bail!("Space-Track username and password are required");
        }
        self.inner.write().await.credentials = Some(Credentials {
            username,
            password: SecretString::from(password),
        });
        Ok(())
    }

    pub async fn clear_credentials(&self) {
        self.inner.write().await.credentials = None;
    }

    pub async fn sync(&self, force: bool) -> anyhow::Result<()> {
        let now = now_unix();
        let (username, password) = {
            let mut inner = self.inner.write().await;
            if inner.syncing {
                anyhow::bail!("space catalog synchronization is already running");
            }
            if !force && has_recent_success(inner.last_successful_sync_unix, now) {
                anyhow::bail!("Space-Track synchronization is limited to once per hour");
            }
            let credentials = inner
                .credentials
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Space-Track credentials are not configured"))?;
            let values = (
                credentials.username.clone(),
                credentials.password.expose_secret().to_owned(),
            );
            inner.syncing = true;
            inner.error = None;
            values
        };
        let result = self.fetch(&username, &password).await;
        let mut inner = self.inner.write().await;
        inner.syncing = false;
        match result {
            Ok(snapshot) => {
                if let Some(parent) = Path::new(CACHE_PATH).parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(CACHE_PATH, serde_json::to_vec(&snapshot)?).await?;
                let synced_unix = snapshot.synced_unix;
                inner.latest_checksum = Some(snapshot.checksum.clone());
                inner.snapshots.insert(snapshot.checksum.clone(), snapshot);
                inner.last_successful_sync_unix = Some(synced_unix);
                Ok(())
            }
            Err(error) => {
                inner.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    async fn fetch(&self, username: &str, password: &str) -> anyhow::Result<SpaceCatalogSnapshot> {
        let login = self
            .client
            .post("https://www.space-track.org/ajaxauth/login")
            .form(&[("identity", username), ("password", password)])
            .send()
            .await
            .map_err(|error| {
                anyhow::anyhow!("Could not reach Space-Track to authenticate: {error}")
            })?;
        if !login.status().is_success() {
            anyhow::bail!(space_track_http_error("authentication", login.status()));
        }
        if login.url().path().contains("login") {
            anyhow::bail!("Space-Track rejected the supplied credentials or the account is not authorized. Verify the username, password, and account access, then try again.");
        }
        let response = self.client.get(GP_URL).send().await.map_err(|error| {
            anyhow::anyhow!("Could not reach Space-Track to download the GP catalog: {error}")
        })?;
        if !response.status().is_success() {
            anyhow::bail!(space_track_http_error(
                "catalog download",
                response.status()
            ));
        }
        let mut objects: Vec<Value> = response.json().await.map_err(|error| {
            anyhow::anyhow!(
                "Space-Track returned a catalog response that could not be read: {error}"
            )
        })?;
        if objects.is_empty() {
            anyhow::bail!("Space-Track returned an empty GP catalog");
        }
        for object in &mut objects {
            if let Some(map) = object.as_object_mut() {
                let object_type = map
                    .get("OBJECT_TYPE")
                    .and_then(Value::as_str)
                    .unwrap_or("UNKNOWN");
                map.insert("SIDC".into(), Value::String(space_sidc(object_type).into()));
            }
        }
        let bytes = serde_json::to_vec(&objects)?;
        let checksum = format!("{:x}", Sha256::digest(&bytes));
        Ok(SpaceCatalogSnapshot {
            synced_unix: now_unix(),
            source: GP_URL.into(),
            checksum,
            objects,
        })
    }

    pub async fn status(&self) -> SpaceCatalogStatus {
        let inner = self.inner.read().await;
        let latest = inner
            .latest_checksum
            .as_ref()
            .and_then(|checksum| inner.snapshots.get(checksum));
        let age = latest.map(|snapshot| now_unix().saturating_sub(snapshot.synced_unix));
        SpaceCatalogStatus {
            setup_auth_required: false,
            remembered_credentials: false,
            configured: inner.credentials.is_some(),
            syncing: inner.syncing,
            usable: age.is_some_and(|seconds| seconds <= MAX_AGE_SECONDS),
            stale: age.is_some_and(|seconds| seconds > MIN_SYNC_INTERVAL_SECONDS),
            synced_unix: latest.map(|snapshot| snapshot.synced_unix),
            age_seconds: age,
            object_count: latest.map_or(0, |snapshot| snapshot.objects.len()),
            checksum: latest.map(|snapshot| snapshot.checksum.clone()),
            error: inner.error.clone(),
        }
    }

    pub async fn snapshot(&self, checksum: &str) -> Option<SpaceCatalogSnapshot> {
        self.inner.read().await.snapshots.get(checksum).cloned()
    }
}

fn space_track_http_error(stage: &str, status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => format!(
            "Space-Track denied {stage} (HTTP {}). Verify the username, password, and that the account is authorized for GP catalog access.",
            status.as_u16()
        ),
        429 => format!(
            "Space-Track rate-limited the {stage} request (HTTP 429). Wait before trying again; this failed request does not start this application's one-hour sync cooldown."
        ),
        500..=599 => format!(
            "Space-Track is currently unavailable during {stage} (HTTP {}). Try again later; this failed request does not start this application's one-hour sync cooldown.",
            status.as_u16()
        ),
        _ => format!(
            "Space-Track {stage} failed with HTTP {} ({}). Check the service and account access, then try again.",
            status.as_u16(),
            status.canonical_reason().unwrap_or("unknown status")
        ),
    }
}

fn has_recent_success(last_successful_sync_unix: Option<u64>, now: u64) -> bool {
    last_successful_sync_unix
        .is_some_and(|last| now.saturating_sub(last) < MIN_SYNC_INTERVAL_SECONDS)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn space_sidc(object_type: &str) -> &'static str {
    match object_type {
        "PAYLOAD" => "100305000011010000000000000000",
        "ROCKET BODY" => "100305000011020000000000000000",
        "DEBRIS" => "100305000011030000000000000000",
        _ => "100305000000000000000000000000",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_messages_explain_auth_and_rate_limit_failures() {
        let denied = space_track_http_error("authentication", reqwest::StatusCode::UNAUTHORIZED);
        assert!(denied.contains("username, password"));
        assert!(denied.contains("HTTP 401"));

        let limited =
            space_track_http_error("catalog download", reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert!(limited.contains("does not start"));
    }

    #[test]
    fn only_a_successful_sync_starts_the_cooldown() {
        assert!(!has_recent_success(None, 10_000));
        assert!(has_recent_success(Some(9_999), 10_000));
        assert!(!has_recent_success(
            Some(10_000 - MIN_SYNC_INTERVAL_SECONDS),
            10_000
        ));
    }
}
