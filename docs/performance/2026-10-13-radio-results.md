# Short terrain-radio measurements — 2026-10-13

The final release benchmark enables sensing and opt-in radio propagation in
Regional Joint Campaign and Global Crisis. Regional uses six checksummed,
synthetic flat HGT tiles; Global uses Earth obstruction without terrain. This is
software responsiveness evidence, not real-terrain or RF accuracy acceptance.

```sh
TERRAIN_DIR=<isolated-synthetic-terrain> target/release/world-at-war-server \
  --benchmark --radio --ticks 30 --warmup 5 --repeats 2 --check
```

Each repeat has five warmup and thirty measured game-second cycles. Every
authored role is projected and serialized each cycle. There is no HTTP gateway,
wall-clock frame pacing, server authority/AI/audit workload, live provider,
PostgreSQL or browser renderer. Other validation ran concurrently; these timings
are indicative and do not establish an isolated before/after improvement or
sustained 60 Hz throughput.

| Scenario | Repeat | Cycle p95 (ms) | Cycle max (ms) |
| --- | --- | --- | --- |
| regional-campaign.v1 | 1 | 44.133 | 59.938 |
| regional-campaign.v1 | 2 | 40.954 | 61.441 |
| global-crisis.v2 | 1 | 3.043 | 3.438 |
| global-crisis.v2 | 2 | 2.995 | 3.034 |

Both regional repeats pass the existing 250 ms p95 / 1,000 ms maximum core cycle
budget. Repeated role-projection hashes agree within each scenario. The raw
[final results](2026-10-13-radio/core.json) include terrain pins, propagation
enablement, seeds, versions and stage timings. The
[source fingerprint](2026-10-13-radio/source-fingerprint.json) identifies the
uncommitted source and scenario fixtures atop `3a2f9bd`; the pinned c3mesh
revision is unchanged. [Initial measurements](2026-10-13-radio/initial-core.json)
are retained as exploratory evidence from before the lifecycle integration fix.

Functional acceptance is covered by deterministic ridge/relay, partial reassembly,
metadata, expiry, return-path, visibility and snapshot tests, plus desktop/phone
real-server browser flows. All 265 fast Rust tests, 19 isolated PostgreSQL tests,
48 frontend tests and 66 browser cases pass. The dedicated PostgreSQL 18.6 test
instance was extracted under `/tmp`, used only for isolated test databases, and
stopped after verification; the development database was untouched.

Real-SRTM reference checks and long endurance runs remain unperformed. No
unattended soak default or durable performance matrix was executed.
