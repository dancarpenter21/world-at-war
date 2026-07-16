use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use sim_catalog::airport::{
    load_airport_snapshot, sync_airport_snapshot, AirportCatalogSnapshot, AirportSyncConfig,
    DEFAULT_REFRESH_MAX_AGE_SECONDS,
};
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AirportCatalogService {
    inner: Arc<RwLock<Inner>>,
    config: Arc<AirportSyncConfig>,
    refresh_max_age_seconds: u64,
}

struct Inner {
    snapshot: Option<Arc<AirportCatalogSnapshot>>,
    syncing: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AirportCatalogStatus {
    pub setup_auth_required: bool,
    pub usable: bool,
    pub syncing: bool,
    pub stale: bool,
    pub degraded: bool,
    pub synced_unix: Option<u64>,
    pub age_seconds: Option<u64>,
    pub airport_count: usize,
    pub runway_count: usize,
    pub checksum: Option<String>,
    pub degraded_sources: Vec<String>,
    pub error: Option<String>,
}

impl AirportCatalogService {
    pub async fn load() -> Self {
        let config = Arc::new(AirportSyncConfig::from_env());
        let refresh_max_age_seconds = std::env::var("AIRPORT_REFRESH_MAX_AGE_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_REFRESH_MAX_AGE_SECONDS);
        let (snapshot, error) = match load_airport_snapshot(&config).await {
            Ok(snapshot) => (snapshot.map(Arc::new), None),
            Err(error) => (
                None,
                Some(format!("cached airport catalog is invalid: {error}")),
            ),
        };
        Self {
            inner: Arc::new(RwLock::new(Inner {
                snapshot,
                syncing: false,
                error,
            })),
            config,
            refresh_max_age_seconds,
        }
    }

    pub async fn refresh_if_stale(&self) {
        if self.status().await.stale {
            let _ = self.sync(false).await;
        }
    }

    pub async fn sync(&self, force: bool) -> anyhow::Result<()> {
        let previous = {
            let mut inner = self.inner.write().await;
            if inner.syncing {
                anyhow::bail!("airport catalog synchronization is already running");
            }
            if !force
                && inner.snapshot.as_ref().is_some_and(|snapshot| {
                    now_unix().saturating_sub(snapshot.synced_unix) < self.refresh_max_age_seconds
                })
            {
                return Ok(());
            }
            inner.syncing = true;
            inner.error = None;
            inner.snapshot.as_deref().cloned()
        };
        let result = sync_airport_snapshot(&self.config, previous.as_ref()).await;
        let mut inner = self.inner.write().await;
        inner.syncing = false;
        match result {
            Ok(snapshot) => {
                inner.snapshot = Some(Arc::new(snapshot));
                Ok(())
            }
            Err(error) => {
                inner.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub async fn snapshot(&self) -> Option<Arc<AirportCatalogSnapshot>> {
        self.inner.read().await.snapshot.clone()
    }

    pub async fn status(&self) -> AirportCatalogStatus {
        let inner = self.inner.read().await;
        let age = inner
            .snapshot
            .as_ref()
            .map(|snapshot| now_unix().saturating_sub(snapshot.synced_unix));
        let runway_count = inner.snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .airports
                .iter()
                .map(|airport| airport.runways.len())
                .sum()
        });
        AirportCatalogStatus {
            setup_auth_required: false,
            usable: inner.snapshot.is_some(),
            syncing: inner.syncing,
            stale: inner.snapshot.is_none()
                || age.is_some_and(|age| age >= self.refresh_max_age_seconds),
            degraded: inner
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| !snapshot.degraded_sources.is_empty()),
            synced_unix: inner.snapshot.as_ref().map(|snapshot| snapshot.synced_unix),
            age_seconds: age,
            airport_count: inner
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.airports.len()),
            runway_count,
            checksum: inner
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.checksum.clone()),
            degraded_sources: inner
                .snapshot
                .as_ref()
                .map_or_else(Vec::new, |snapshot| snapshot.degraded_sources.clone()),
            error: inner.error.clone(),
        }
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
