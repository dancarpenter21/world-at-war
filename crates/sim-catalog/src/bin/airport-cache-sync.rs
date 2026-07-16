use sim_catalog::airport::{load_airport_snapshot, sync_airport_snapshot, AirportSyncConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AirportSyncConfig::from_env();
    let previous = load_airport_snapshot(&config)
        .await
        .unwrap_or_else(|error| {
            eprintln!("Ignoring invalid existing airport cache: {error}");
            None
        });
    let snapshot = sync_airport_snapshot(&config, previous.as_ref()).await?;
    let runway_count: usize = snapshot
        .airports
        .iter()
        .map(|airport| airport.runways.len())
        .sum();
    println!(
        "Airport cache ready: {} facilities, {} runways, checksum {}{}",
        snapshot.airports.len(),
        runway_count,
        snapshot.checksum,
        if snapshot.degraded_sources.is_empty() {
            String::new()
        } else {
            format!(" (degraded: {})", snapshot.degraded_sources.join("; "))
        }
    );
    Ok(())
}
