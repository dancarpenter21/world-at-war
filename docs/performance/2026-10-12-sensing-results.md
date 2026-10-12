# Shared clock and sensing verification — 2026-10-12

These measurements apply to the source fingerprint below, before the subsequent
60 Hz fractional-frame change. They do not certify that later loop.

Functional verification passes. The final five-minute volatile regional run meets
the existing cycle and reconnect budgets on explicitly synthetic flat terrain.
Durable regional performance acceptance remains open: the 75-second PostgreSQL
smoke is valid but misses p95, and the five-minute attempt times out before its
measurement window completes. Neither establishes durable acceptance.

## Results

| Final workload | Measurement | Valid | Cycle p95 | Cycle maximum | Result |
| --- | --- | --- | --- | --- | --- |
| Regional, volatile, 11 clients | 60 s warmup + 300 s | Yes | 212.4 ms | 300.3 ms | Cycle and reconnect budgets met |
| Regional, PostgreSQL, 11 clients | 15 s warmup + 75 s | Yes | 596.7 ms | 696.5 ms | Cycle p95 misses 250 ms; reconnects pass |
| Regional, PostgreSQL, 11 clients | 60 s warmup + intended 300 s | **No** | 1,037.1 ms, partial window | 4,695.0 ms, partial window | Client timeout; incomplete measurement |

The cycle target is p95 ≤250 ms and maximum ≤1,000 ms. Reconnect maximum is
≤3,000 ms. The valid runs have zero client errors, failed durable operations and
dropped trace samples. The failed longer run has zero dropped trace samples but
is **not acceptance evidence**. Its largest commit is 4,630.0 ms, including a
4,558.4 ms flush. This locates the observed stall; it does not establish the
underlying storage/checkpoint cause. Cycle traces include maintenance while games
are running, as well as advancing ticks.

The offline release benchmark uses 30 warmup and 120 measured ticks, two repeats,
seed 12787750. All four legacy sample hashes match the previous checkpoint.
Upgraded hashes repeat exactly. Regional upgraded tick p95 is 108.9–117.3 ms;
Global Crisis is 2.4–2.6 ms. The new model is more expensive than the legacy
regional 5.1–5.7 ms workload; these are different sensing workloads.

Exploratory runs exposed large overlapping reports and ambiguous track growth.
The implementation now bounds pending report batches, ranks gated associations by
normalized Gaussian likelihood, and retains ambiguous same-class emissions as
bearings. The earlier approximately 72 MB snapshot at tick 32 becomes approximately
4.3 MB at tick 53 in the valid durable smoke. This is a diagnostic comparison at
different ticks, not a memory or endurance acceptance result. Earlier failed runs
are preserved under `tmp/performance/sensing-20261012`; they are not replaced by
successful samples.

## Evidence and environment

- [Volatile result and metadata](2026-10-12-sensing/volatile.json)
- [Valid PostgreSQL smoke](2026-10-12-sensing/postgres-smoke.json)
- [Failed PostgreSQL attempt](2026-10-12-sensing/postgres-attempt.json)
- [Legacy benchmark](2026-10-12-sensing/legacy-core.json) and [upgraded benchmark](2026-10-12-sensing/sensing-core.json)
- [Changed-source and binary fingerprints](2026-10-12-sensing/source-fingerprint.json)
- [Shared speed and controller weather screenshot](2026-10-12-sensing/sensing-controls.png)

Native measurements use an AMD Ryzen 7 9800X3D, 16 logical CPUs, Rust 1.96 and
Node 24.15.0 on Linux. PostgreSQL is isolated 17.11 on a fresh Docker named volume
with fsync, synchronous_commit and full_page_writes enabled. Its image digest,
settings, hardware, binary SHA-256, terrain manifest and measurement timestamps
are recorded in metadata. The checkout is dirty, based on `9d0a0b5`; the source
fingerprint identifies this implementation. No competing builds or browser suite
ran during these final measurements. Existing development services were left
untouched. Full traces, logs and original summaries remain in the artifact
directories named in the JSON files.

Terrain consists of six labelled synthetic flat 1201-sample HGT tiles. It is not
an installed SRTMGL1 dataset and cannot validate real terrain masking, reference
terrain budgets or long-term stability. The map still presents its existing
ellipsoid. No long soak was run. Real terrain setup, fidelity limits and
stopped-server migration instructions are in the
[operation guide](../SENSING_IMPLEMENTATION.md).

## Verification

Passed: 240 fast Rust tests, 16 isolated PostgreSQL tests, 46 frontend tests,
62 Chromium browser tests and 16 fast harness tests including real-container
interruption cleanup. Formatting, strict Clippy, Rust 1.89 compatibility,
TypeScript production/browser checking, frontend production build, startup budget,
dependency-pin verification and production/development Compose configuration pass.
Startup JavaScript is 269.2 KiB against 350 KiB. Both production Docker images
build, and isolated command/observer smoke checks pass. Owned validation containers
and networks were removed. No existing database was migrated.

Reproduction uses the commands in [LOAD_TESTING.md](LOAD_TESTING.md), supplying an
absolute `TERRAIN_DIR`, fresh output directories and the preserved binary/source.
`--benchmark --sensing` selects the upgraded offline workload. Preserve failed
reports and keep synthetic and real-terrain results separately labelled.
