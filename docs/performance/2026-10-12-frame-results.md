# Committed-frame performance baseline — 2026-10-12

The server now measures successful frame commits separately from pre-persistence
processing and maintenance. Role diagnostics show committed frame rate and achieved
game speed. This is a baseline and measurement increment; pacing, checkpoint policy,
atomic commits and recovery formats remain unchanged.

Durable 60 Hz acceptance remains open. At requested 1×, PostgreSQL delivers
10.25–16.17 Hz for regional, 18.91–19.31 Hz for global, and 2.92–3.26 Hz per
game for mixed workloads. Regional and mixed reconnects also exceed three
seconds. The measured follow-up priorities are repeated full-state encoding,
HTTP checkpoint/mutation work under the shared durability gate, and SQL volume.

## Reference results

24/24 final short runs are valid. Each reference run uses 60 seconds
of warmup and 300 measured seconds, seed 12787750, 1× requested speed, and reconnects
every 60 seconds. The matrix repeats in reverse storage-mode order. Mixed workloads
contain three regional and two global games with 50 distinct clients.

| Repetition | Profile / storage | Valid | Per-game committed Hz | Achieved speed | Cycle p95 / max, ms | Reconnect max, ms |
| --- | --- | --- | --- | --- | --- | --- |
| forward | [regional / volatile](2026-10-12-frames/forward-regional-volatile.json) | Yes | 57.30 | 0.955× | 1.1 / 219.3 | 63.3 |
| forward | [global / volatile](2026-10-12-frames/forward-global-volatile.json) | Yes | 60.00 | 1.000× | 0.9 / 5.3 | 21.2 |
| forward | [mixed / volatile](2026-10-12-frames/forward-mixed-volatile.json) | Yes | 50.99 | 0.850× | 20.4 / 215.6 | 30.6 |
| forward | [regional / postgres](2026-10-12-frames/forward-regional-postgres-retry-1.json) | Yes | 16.17 | 0.270× | 154.8 / 630.4 | 5061.6 |
| forward | [global / postgres](2026-10-12-frames/forward-global-postgres.json) | Yes | 19.31 | 0.322× | 50.8 / 100.6 | 464.1 |
| forward | [mixed / postgres](2026-10-12-frames/forward-mixed-postgres-retry-3.json) | Yes | 3.26 | 0.054× | 275.5 / 531.0 | 5216.9 |
| reverse | [regional / postgres](2026-10-12-frames/reverse-regional-postgres.json) | Yes | 10.25 | 0.171× | 206.1 / 780.1 | 9601.8 |
| reverse | [global / postgres](2026-10-12-frames/reverse-global-postgres.json) | Yes | 18.91 | 0.315× | 51.6 / 101.0 | 399.4 |
| reverse | [mixed / postgres](2026-10-12-frames/reverse-mixed-postgres.json) | Yes | 2.92 | 0.049× | 296.0 / 544.3 | 10373.6 |
| reverse | [regional / volatile](2026-10-12-frames/reverse-regional-volatile.json) | Yes | 57.10 | 0.952× | 1.2 / 211.1 | 183.7 |
| reverse | [global / volatile](2026-10-12-frames/reverse-global-volatile.json) | Yes | 60.00 | 1.000× | 0.9 / 4.4 | 19.6 |
| reverse | [mixed / volatile](2026-10-12-frames/reverse-mixed-volatile.json) | Yes | 50.90 | 0.848× | 21.2 / 227.5 | 46.5 |

The 60 Hz value is a target, not a newly invented tolerance. The existing regional
cycle limits are p95 ≤250 ms and maximum ≤1,000 ms; reconnect maximum is ≤3,000 ms.
Passing those older budgets does not establish sustained 60 Hz. Reconnect figures
cover only the sampled clients recorded in each summary. Most frames at 1×
only perform fractional work, so their cycle percentiles are not directly comparable
with historical one-game-second tick percentiles. Each summary includes separate
fractional and whole-second processing metrics and zero-progress wall-time buckets.

## Durability and growth

| Repetition / profile | Commit p95, ms | Flush max, ms | Checkpoints / wall second | Journal KiB / wall second | Submitted snapshots KiB / wall second | Sampled DB / WAL growth, MiB | Sampled seconds |
| --- | --- | --- | --- | --- | --- | --- | --- |
| forward / regional | 24.3 | 33.5 | 0.67 | 9691.6 | 12607.9 | 950.4 / 1659.8 | 295.6 |
| forward / global | 4.0 | 12.2 | 0.78 | 4880.5 | 8306.2 | 65.2 / 186.0 | 295.0 |
| forward / mixed | 212.8 | 31.7 | 2.04 | 14812.2 | 16723.9 | 739.9 / 1230.5 | 294.8 |
| reverse / regional | 63.2 | 51.3 | 0.57 | 13023.3 | 15266.9 | 1034.5 / 1868.9 | 296.0 |
| reverse / global | 4.0 | 14.8 | 0.77 | 4950.3 | 8199.6 | 65.1 / 184.6 | 295.9 |
| reverse / mixed | 307.9 | 41.7 | 1.99 | 16602.9 | 18295.7 | 806.7 / 1357.4 | 295.9 |

Byte submissions are encoded data sent to PostgreSQL, not on-disk size. Database
and WAL growth use first/last in-window samples over the stated shorter interval.
Summaries also report totals per aggregate committed game second; different
achieved speeds produce different game-time workloads. Nested timing spans must
not be summed as independent costs. Shared transaction costs stay server-wide.

## Measured optimization priorities

1. **Reduce repeated snapshot encoding and hashing.** Global durable boundaries
   average approximately three encodes at 11.6–11.9 ms each, plus a 4.35 ms hash.
   That is about 39–40 ms before most SQL work. Profile reusable committed rollback
   bytes and duplicate state serialization first, preserving canonical hashes,
   atomic rollback, fencing and both legacy and fractional replay.
2. **Reduce checkpoint/mutation cost and shared-gate contention.** Regional HTTP
   mutation boundaries reach 1.16–1.73 seconds; simulation gate waits reach
   4.34–5.04 seconds. Mixed gate waits reach 5.49–7.14 seconds. Consequently,
   committed-frame gaps reach 4.44–5.27 seconds regionally and 5.73–7.44 seconds
   in mixed runs even though cycle maxima stay below one second. Review full-state
   HTTP checkpoints and the current every-60-boundary checkpoint cadence using
   explicit recovery/compatibility tests; this increment changes neither policy.
3. **Reduce submitted bytes and SQL work.** Regional journal/snapshot submissions
   reach roughly 9.9–13.3 / 12.9–15.6 MB per wall second. Journal totals include
   full HTTP checkpoint payloads, not just small frame inputs. Mixed SQL p95 is
   207.6–299.3 ms. Final flush maxima in these runs are only 12.2–51.3 ms; the
   historical flush-stall explanation does not establish this baseline's steady
   bottleneck, and underlying PostgreSQL/storage causes remain unproven.
4. **Retain separate whole-second sensing measurements.** Fractional frames are
   cheap in volatile mode, while regional whole-second work reduces achieved
   pacing to roughly 95%, and shared mixed pacing to roughly 85%. Re-measure those
   costs after durability work, then add real-terrain reference evidence separately.

These priorities follow observed spans, byte counts and source inspection; they
are directions for the next optimization increment, not completed optimizations.

## Brief speed checks

One global game with 13 clients runs for 15 seconds warmup and 45 measured seconds
at each remaining speed. These checks exercise host selection, fractional progress
and telemetry; they do not certify regional or mixed performance at those speeds.

| Requested speed | Volatile Hz / achieved speed / valid | PostgreSQL Hz / achieved speed / valid |
| --- | --- | --- |
| 0.25× | [60.00 Hz / 0.25× / Yes](2026-10-12-frames/speed-0_25-volatile.json) | [19.89 Hz / 0.08× / Yes](2026-10-12-frames/speed-0_25-postgres.json) |
| 2× | [60.00 Hz / 2.00× / Yes](2026-10-12-frames/speed-2-volatile.json) | [18.58 Hz / 0.62× / Yes](2026-10-12-frames/speed-2-postgres.json) |
| 5× | [60.00 Hz / 5.00× / Yes](2026-10-12-frames/speed-5-volatile.json) | [18.31 Hz / 1.53× / Yes](2026-10-12-frames/speed-5-postgres.json) |
| 10× | [60.00 Hz / 10.00× / Yes](2026-10-12-frames/speed-10-volatile.json) | [17.31 Hz / 2.89× / Yes](2026-10-12-frames/speed-10-postgres.json) |
| 30× | [60.00 Hz / 30.00× / Yes](2026-10-12-frames/speed-30-volatile.json) | [18.36 Hz / 9.18× / Yes](2026-10-12-frames/speed-30-postgres.json) |
| 60× | [60.00 Hz / 60.00× / Yes](2026-10-12-frames/speed-60-volatile.json) | [17.80 Hz / 17.80× / Yes](2026-10-12-frames/speed-60-postgres.json) |

## Evidence and limits

- [Initial source and binary fingerprint](2026-10-12-frames/source-fingerprint.json), [final harness fingerprint](2026-10-12-frames/source-fingerprint-final-harness.json), and [explicit short-run matrix](2026-10-12-frames/matrix.json).
- [Excluded interrupted and incomplete attempts](2026-10-12-frames/excluded-attempts.json).
- [Browser diagnostics screenshot](2026-10-12-frames/committed-frame-diagnostics.png).
- Per-run links contain metadata and complete summaries. Raw traces/logs remain in the named ignored artifact directories.

Measurements use the fingerprints above, based on `59c6104`, with a
dirty checkout. No competing builds, tests or browser suite ran during the final
accepted profiles. Completed valid profiles were retained after harness-only
freshness/cleanup corrections; affected durable profiles use fresh retry paths.
The server binary and configured request workload remain unchanged; new-run
metadata records exact harness hashes. Recovery bookkeeping was corrected to
keep clients offline until socket opening completes. Existing development services
and databases were left untouched. PostgreSQL
uses fresh isolated Docker named volumes, the same pinned image, and enabled fsync,
synchronous commit and full-page writes. Metadata records hardware, toolchain,
PostgreSQL settings, terrain manifest, exact binary hash and measurement windows.

Regional terrain is six explicitly synthetic flat 1201-sample HGT tiles from
`tmp/sensing-validation/terrain`. Checksums and dimensions were verified before
measurement. These results do not establish real SRTM masking or reference-terrain
acceptance. Browser rendering, roadmap-scale loads, prolonged growth and long-soak
acceptance remain outside this increment. No long soak or development-database
migration was run.

An earlier matrix under `tmp/performance/frame-baseline-20261012` remains preserved.
Its durable regional attempt stopped because an old unconditional 15-second stream
watchdog treated quiet fractional progress as a stalled connection. Successful
commits and host-clock replies continued while network sequences waited for the
next game second. The harness now checks committed clock progress independently
and bounds delivery of known expected game-second updates; observer streams retain
their wall-time freshness check. The subsequent global attempt was interrupted to
apply this correction. Neither incomplete attempt is acceptance evidence.
A later durable reconnect check still required whole-second tick advancement
during a ten-second outage. The corrected check uses operational snapshot frames;
current truth snapshots may keep the same tick and must not regress. Interrupted
setup now aborts requests immediately, and closed output pipes cannot bypass
cleanup. Affected attempts remain preserved and excluded; only owned services
and volumes were removed before retries. Shared clock probes suspend during
planned host outages; a recovering client stays offline until the WebSocket opens
and its clock deadline is reset. Delayed-handshake tests cover that ordering.
All these changes affect validation/recovery bookkeeping or failure cleanup,
not server simulation, persistence or pacing behavior.

## Verification

Passed: 248 fast Rust tests, 17 isolated PostgreSQL tests, 47 frontend tests,
23 fast harness tests including real-container and setup interruption cleanup, and the
updated real-server browser diagnostics regression. Formatting, strict workspace
Clippy, dependency-pin verification, TypeScript/browser checking and production
frontend/release-server builds pass. Startup JavaScript is 270.5 KiB against the
350 KiB budget. Diagnostics stay role-authorized and outside snapshots; injected
commit failure publishes no committed progress and recovery/replay tests pass.

Reproduction commands and metric definitions are in [LOAD_TESTING.md](LOAD_TESTING.md).
Use fresh output directories, the pinned PostgreSQL image and an explicitly labelled
synthetic fixture or separately installed real terrain. Preserve unsuccessful runs.
