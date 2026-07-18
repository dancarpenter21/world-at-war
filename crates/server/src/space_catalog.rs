use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use reqwest::header::CONTENT_TYPE;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

const CACHE_PATH: &str = "data/cache/space-track/latest.json";
const CACHE_TEMP_PATH: &str = "data/cache/space-track/latest.json.tmp";
const MIN_SYNC_INTERVAL_SECONDS: u64 = 3_600;
const DEFAULT_LOGIN_URL: &str = "https://www.space-track.org/ajaxauth/login";
const DEFAULT_GP_URL: &str = "https://www.space-track.org/basicspacedata/query/class/gp/decay_date/null-val/epoch/%3Enow-10/orderby/norad_cat_id/format/json";

#[derive(Clone)]
pub struct SpaceCatalogService {
    inner: Arc<RwLock<Inner>>,
    client: reqwest::Client,
    login_url: String,
    gp_url: String,
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
    pub using_cached_fallback: bool,
    pub synced_unix: Option<u64>,
    pub age_seconds: Option<u64>,
    pub object_count: usize,
    pub checksum: Option<String>,
    pub error: Option<String>,
}

impl SpaceCatalogService {
    pub async fn load() -> anyhow::Result<Self> {
        let snapshot = match tokio::fs::read(CACHE_PATH).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .ok()
                .filter(is_valid_snapshot),
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
            login_url: std::env::var("SPACETRACK_LOGIN_URL")
                .unwrap_or_else(|_| DEFAULT_LOGIN_URL.into()),
            gp_url: std::env::var("SPACETRACK_GP_URL").unwrap_or_else(|_| DEFAULT_GP_URL.into()),
        })
    }

    pub async fn configure_and_sync(
        &self,
        username: String,
        password: String,
    ) -> anyhow::Result<SpaceCatalogStatus> {
        self.configure(username, password).await?;
        self.authenticate().await?;
        match self.sync_catalog(false, false).await {
            Ok(()) => Ok(self.status().await),
            Err(_error) if self.status().await.usable => Ok(self.status().await),
            Err(error) => Err(error),
        }
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

    async fn authenticate(&self) -> anyhow::Result<()> {
        let (username, password) = {
            let inner = self.inner.read().await;
            let credentials = inner
                .credentials
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Space-Track credentials are not configured"))?;
            (
                credentials.username.clone(),
                credentials.password.expose_secret().to_owned(),
            )
        };
        self.authenticate_with(&username, &password).await
    }

    async fn authenticate_with(&self, username: &str, password: &str) -> anyhow::Result<()> {
        let login = self
            .client
            .post(&self.login_url)
            .form(&[("identity", username), ("password", password)])
            .send()
            .await
            .map_err(|error| {
                anyhow::anyhow!("Could not reach Space-Track to authenticate: {error}")
            })?;
        if !login.status().is_success() {
            anyhow::bail!(space_track_http_error("authentication", login.status()));
        }
        Ok(())
    }

    pub async fn sync(&self, force: bool) -> anyhow::Result<()> {
        self.sync_catalog(force, true).await
    }

    async fn sync_catalog(&self, force: bool, authenticate: bool) -> anyhow::Result<()> {
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
        let result = match if authenticate {
            self.fetch(&username, &password).await
        } else {
            self.fetch_catalog().await
        } {
            Ok(snapshot) => persist_snapshot(&snapshot).await.map(|()| snapshot),
            Err(error) => Err(error),
        };
        let mut inner = self.inner.write().await;
        inner.syncing = false;
        match result {
            Ok(snapshot) => {
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
        self.authenticate_with(username, password).await?;
        self.fetch_catalog().await
    }

    async fn fetch_catalog(&self) -> anyhow::Result<SpaceCatalogSnapshot> {
        let response = self
            .client
            .get(&self.gp_url)
            .send()
            .await
            .map_err(|error| {
                anyhow::anyhow!("Could not reach Space-Track to download the GP catalog: {error}")
            })?;
        if !response.status().is_success() {
            anyhow::bail!(space_track_http_error(
                "catalog download",
                response.status()
            ));
        }
        if is_login_path(response.url().path()) {
            anyhow::bail!("Space-Track rejected the supplied credentials or the account is not authorized. Verify the username, password, and account access, then try again.");
        }
        if response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(is_html_content_type)
        {
            anyhow::bail!("Space-Track returned a login or HTML page instead of the GP catalog. The authentication session may not have been established; verify the account credentials and access, then try again.");
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
            source: self.gp_url.clone(),
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
            usable: latest.is_some(),
            stale: age.is_some_and(|seconds| seconds > MIN_SYNC_INTERVAL_SECONDS),
            using_cached_fallback: latest.is_some() && inner.error.is_some(),
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

async fn persist_snapshot(snapshot: &SpaceCatalogSnapshot) -> anyhow::Result<()> {
    if let Some(parent) = Path::new(CACHE_PATH).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(CACHE_TEMP_PATH, serde_json::to_vec(snapshot)?).await?;
    tokio::fs::rename(CACHE_TEMP_PATH, CACHE_PATH).await?;
    Ok(())
}

fn is_valid_snapshot(snapshot: &SpaceCatalogSnapshot) -> bool {
    !snapshot.checksum.is_empty() && !snapshot.objects.is_empty()
}

fn is_login_path(path: &str) -> bool {
    path == "/auth/login" || path.starts_with("/auth/login/")
}

fn is_html_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("text/html"))
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

    fn service_with_snapshot(
        snapshot: SpaceCatalogSnapshot,
        error: Option<&str>,
    ) -> SpaceCatalogService {
        let checksum = snapshot.checksum.clone();
        SpaceCatalogService {
            inner: Arc::new(RwLock::new(Inner {
                credentials: None,
                snapshots: BTreeMap::from([(checksum.clone(), snapshot)]),
                latest_checksum: Some(checksum),
                syncing: false,
                last_successful_sync_unix: None,
                error: error.map(str::to_owned),
            })),
            client: reqwest::Client::new(),
            login_url: DEFAULT_LOGIN_URL.into(),
            gp_url: DEFAULT_GP_URL.into(),
        }
    }

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
    fn only_catalog_redirects_to_the_login_page_are_authentication_failures() {
        assert!(is_login_path("/auth/login"));
        assert!(is_login_path("/auth/login/expired"));
        assert!(!is_login_path("/ajaxauth/login"));
        assert!(!is_login_path("/basicspacedata/query/class/gp/format/json"));
    }

    #[test]
    fn html_catalog_responses_are_not_parsed_as_json() {
        assert!(is_html_content_type("text/html"));
        assert!(is_html_content_type("Text/HTML; charset=UTF-8"));
        assert!(!is_html_content_type("application/json"));
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

    #[tokio::test]
    async fn old_cached_snapshots_remain_usable_and_report_fallback() {
        let service = service_with_snapshot(
            SpaceCatalogSnapshot {
                synced_unix: now_unix().saturating_sub(MIN_SYNC_INTERVAL_SECONDS + 1),
                source: "test".into(),
                checksum: "cached-checksum".into(),
                objects: vec![serde_json::json!({ "NORAD_CAT_ID": "5" })],
            },
            Some("catalog download timed out"),
        );

        let status = service.status().await;
        assert!(status.usable);
        assert!(status.stale);
        assert!(status.using_cached_fallback);
        assert_eq!(status.checksum.as_deref(), Some("cached-checksum"));
    }

    #[test]
    fn empty_cached_snapshots_are_not_usable() {
        assert!(!is_valid_snapshot(&SpaceCatalogSnapshot {
            synced_unix: 0,
            source: "test".into(),
            checksum: "checksum".into(),
            objects: vec![],
        }));
    }
}
