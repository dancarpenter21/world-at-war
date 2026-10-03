# Scenario performance checkpoint — 2026-10-02

These measurements and projection hashes describe the campaign branch before its
2026-10-03 integration with upstream gameplay. The merged implementation retains
campaign transport optimizations but also includes the upstream packet engine,
scoped track identities, training combat, and scenario-specific visibility rules.
Generate a new baseline before comparing performance or projection hashes on main.

Release measurements on WSL Linux x86_64, AMD Ryzen 7 9800X3D, 16 logical CPUs,
Rust 1.96.0. Each run uses seed 12787750, 30 warmup ticks, 120 measured ticks and
two independent repeats. Raw measurements: [before](2026-10-02-before.json) and
[after](2026-10-02-after.json).

| Scenario | Entities / directed links | Before tick p95 | After tick p95 | After role projection p95 |
| --- | --- | --- | --- | --- |
| Regional Joint Campaign | 12 / 78 | 2.65–2.78 ms | 2.44–2.48 ms | 0.010 ms |
| Global Crisis | 64 / 2,496 | 1,010–1,022 ms | 51.5–57.8 ms | 0.049–0.052 ms |

Global Crisis tick p95 fell approximately 94–95%. Its worst measured tick fell
from 1,093 ms to 77 ms. Every role's serialized projection at every tick, including
warmup, has the same SHA-256 digest before and after optimization, across both
repeats. Scenario version, seed, workload sizes and hashes are in the JSON.

The dominant cost was repeated route searches and c3mesh transmission metrics.
The transport now evaluates available directed links once per tick and shares
shortest paths across messages and acknowledgements. Sorted neighbor order is
preserved; the table is discarded before the next geometry/interference update.
Immutable topology indexes remove repeated full-link scans. Report serialization
is reused per sender within a report pass, retired fragment cleanup is batched,
and projections calculate telemetry only for role-visible incoming links.
Full-mesh dissemination, delivery timing, retry limits and visibility rules remain
the same.

## Reproduce and check

From the repository root, with the sibling c3mesh checkout available:

```sh
cargo run --release -p world-at-war-server -- --benchmark \
  --ticks 120 --warmup 30 --repeats 2 --seed 12787750 \
  --check --baseline docs/performance/2026-10-02-before.json > benchmark.json
```

The command runs offline without starting HTTP services, loading credentials,
fetching catalogs or altering a running game. JSON goes to stdout and progress
to stderr. Repeat hashes must agree. `--baseline` also requires matching scenario
version, seed and tick counts and identical projection hashes. `--check` enforces
the regional **simulation-core** p95 <=250 ms and maximum <=1,000 ms guardrail;
it does not certify the complete roadmap budget. Omit `--baseline` when deliberately
changing simulation semantics, and review new hashes before updating fixtures.
Measure without concurrent builds or other heavy workloads.

This workload includes ECS, interference, packet transport, reports, and projection
and serialization for all 10 regional / 15 global roles each tick. Projection
work is measured separately from simulation ticks. It excludes server AI,
authority and audit processing, gateway/network latency, live catalogs, pacing,
and browser/GPU work. Uncompressed snapshot sizes and per-role visible queue
peaks are reported. It does not model 50 concurrent clients or the roadmap's
1,000-platform / 10,000-background-object workload. Those load/soak tests and
compressed snapshot/delta budgets remain open.

## Runtime diagnostics

The map's **Diagnostics** button shows connection freshness, state request time,
map reconciliation time, full server tick latest/p95 and overruns, projection
latest/p95, uncompressed state size and queues visible to the held role. It
separates a paused simulation from failed state requests and delayed progress.
Map processing measures JavaScript entity reconciliation, not GPU rendering.

`GET /v1/games/{game_id}/diagnostics?player_id=...&role_id=...` uses the same
role authorization as `/state`. Samples are bounded to 120 per metric and are
observational only. Full server ticks include AI, authority resolution,
simulation, delivery handling and planning reports. Projection samples describe
HTTP `/state` responses; other projection consumers are not included. Diagnostics
also exposes the latest simulation-stage timings and scheduling delay. Queue
totals include only links present in that role's projection, never global queues
or hidden entity/contact identifiers. Timing samples are excluded from canonical
simulation state and deterministic hashes.
