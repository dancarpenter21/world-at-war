//! Offline, repeatable release benchmark; no HTTP services, credentials, or provider requests.
use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::Instant;

fn number(args: &[String], flag: &str, default: u64) -> anyhow::Result<u64> {
    args.iter()
        .position(|arg| arg == flag)
        .map_or(Ok(default), |index| {
            args.get(index + 1)
                .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?
                .parse()
                .map_err(Into::into)
        })
}

pub(super) fn run(args: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        !cfg!(debug_assertions),
        "benchmarks require cargo run --release"
    );
    let mut seen = BTreeSet::new();
    let mut options = args.iter();
    while let Some(flag) = options.next() {
        anyhow::ensure!(seen.insert(flag), "duplicate option {flag}");
        match flag.as_str() {
            "--check" => (),
            "--ticks" | "--warmup" | "--seed" | "--repeats" | "--baseline" => {
                anyhow::ensure!(options.next().is_some(), "missing value for {flag}");
            }
            _ => anyhow::bail!("unknown benchmark option {flag}"),
        }
    }
    let baseline: Option<serde_json::Value> = args
        .iter()
        .position(|arg| arg == "--baseline")
        .map(|index| -> anyhow::Result<_> {
            Ok(serde_json::from_slice(&std::fs::read(&args[index + 1])?)?)
        })
        .transpose()?;
    let ticks = number(args, "--ticks", 120)?;
    let warmup = number(args, "--warmup", 30)?;
    let seed = number(args, "--seed", 12_787_750)?;
    let repeats = number(args, "--repeats", 2)?;
    anyhow::ensure!(
        ticks > 0 && repeats > 0 && ticks <= 100_000 && warmup <= 100_000 && repeats <= 20,
        "invalid benchmark length"
    );
    let mut results = vec![];
    for scenario in [regional_campaign_scenario(), global_crisis_scenario()] {
        let mut reference = None;
        for repeat in 0..repeats {
            let mut simulation = scenario.spawn_with_seed(seed)?;
            let mut timings = vec![];
            let mut projection_ms = vec![];
            let mut serialization_ms = vec![];
            let mut snapshot_bytes = vec![];
            let mut queue_packets = 0;
            let mut queue_bytes = 0;
            let mut digest = Sha256::new();
            let started = Instant::now();
            for index in 0..warmup + ticks {
                simulation.step();
                let measured = index >= warmup;
                if measured {
                    timings.push(simulation.last_step_timings());
                }
                for role in &scenario.authority.roles {
                    let started = Instant::now();
                    let projection = simulation.projection_for(role.location_unit_id, role.side);
                    if measured {
                        projection_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    let started = Instant::now();
                    let bytes = serde_json::to_vec(&projection)?;
                    if measured {
                        serialization_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                        snapshot_bytes.push(bytes.len() as f64);
                        queue_packets = queue_packets.max(
                            projection
                                .communication_links
                                .iter()
                                .map(|link| link.queued_packets)
                                .sum::<usize>(),
                        );
                        queue_bytes = queue_bytes.max(
                            projection
                                .communication_links
                                .iter()
                                .map(|link| link.queued_bytes)
                                .sum::<usize>(),
                        );
                    }
                    digest.update(role.id.as_bytes());
                    digest.update((bytes.len() as u64).to_le_bytes());
                    digest.update(bytes);
                }
                // Like the server, consume external transport events after each tick.
                simulation.drain_deliveries();
            }
            let hash = format!("{:x}", digest.finalize());
            if let Some(expected) = &reference {
                anyhow::ensure!(
                    expected == &hash,
                    "non-deterministic role projections for {}",
                    scenario.id
                );
            }
            if let Some(baseline) = &baseline {
                let matching: Vec<_> = baseline["results"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("baseline missing results"))?
                    .iter()
                    .filter(|result| {
                        result["scenario"] == scenario.id
                            && result["scenario_version"] == scenario.version
                            && result["seed"] == seed
                            && result["warmup_ticks"] == warmup
                            && result["measured_ticks"] == ticks
                    })
                    .collect();
                anyhow::ensure!(
                    !matching.is_empty(),
                    "no matching baseline workload for {}",
                    scenario.id
                );
                anyhow::ensure!(
                    matching
                        .iter()
                        .all(|result| result["projection_sha256"] == hash),
                    "role projection hash differs from baseline for {}",
                    scenario.id
                );
            }
            reference = Some(hash.clone());
            let total = summary(timings.iter().map(|sample| sample.total_ms).collect());
            let regional_budget = scenario.id == "regional-campaign.v1";
            let budget_met = total.p95 <= 250.0 && total.max <= 1000.0;
            results.push(json!({ "scenario": scenario.id, "scenario_version": scenario.version, "seed": seed,
                "repeat": repeat + 1, "warmup_ticks": warmup, "measured_ticks": ticks, "units": scenario.units.len(),
                "roles_projected_per_tick": scenario.authority.roles.len(), "links": scenario.communication_links.len(),
                "elapsed_seconds": started.elapsed().as_secs_f64(), "projection_sha256": hash,
                "simulation_tick_ms": total,
                "ecs_ms": summary(timings.iter().map(|s| s.ecs_ms).collect()),
                "interference_ms": summary(timings.iter().map(|s| s.interference_ms).collect()),
                "transport_ms": summary(timings.iter().map(|s| s.transport_ms).collect()),
                "transmission_ms": summary(timings.iter().map(|s| s.transmission_ms).collect()),
                "network_advance_ms": summary(timings.iter().map(|s| s.network_advance_ms).collect()),
                "reassembly_ms": summary(timings.iter().map(|s| s.reassembly_ms).collect()),
                "reports_ms": summary(timings.iter().map(|s| s.reports_ms).collect()),
                "role_projection_ms": summary(projection_ms), "role_serialization_ms": summary(serialization_ms),
                "snapshot_bytes_uncompressed": summary(snapshot_bytes), "peak_visible_queue_packets": queue_packets,
                "peak_visible_queue_bytes": queue_bytes, "regional_simulation_budget_met": regional_budget.then_some(budget_met) }));
            eprintln!(
                "{} repeat {}: tick p95 {:.3} ms, max {:.3} ms",
                scenario.id,
                repeat + 1,
                total.p95,
                total.max
            );
        }
    }
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find_map(|line| line.strip_prefix("model name\t: ").map(str::to_owned));
    let metadata = json!({ "schema_version": 1, "profile": "release", "cpu": cpu,
        "logical_cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "os": std::env::consts::OS, "arch": std::env::consts::ARCH,
        "rustc": std::process::Command::new("rustc").arg("--version").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()),
        "workload": "Seeded scenario ECS, communications and reports; every scenario role projected/serialized each tick. No gateway, wall-clock pacing, server AI/authority/audit, live catalogs or browser rendering.",
        "results": results });
    println!("{}", serde_json::to_string_pretty(&metadata)?);
    if args.iter().any(|arg| arg == "--check") {
        anyhow::ensure!(
            metadata["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["regional_simulation_budget_met"] != false),
            "regional simulation tick budget exceeded"
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Serialize)]
struct Summary {
    samples: usize,
    mean: f64,
    p50: f64,
    p95: f64,
    max: f64,
}
fn summary(mut samples: Vec<f64>) -> Summary {
    samples.sort_by(f64::total_cmp);
    let count = samples.len();
    Summary {
        samples: count,
        mean: samples.iter().sum::<f64>() / count as f64,
        p50: samples[(count * 50).div_ceil(100) - 1],
        p95: samples[(count * 95).div_ceil(100) - 1],
        max: samples[count - 1],
    }
}
