# Communications audit write-path optimization — 2026-10-11

## Implementation and correctness

Audit preparation now borrows captured events and encodes private, role-local and
runtime JSON once using `serde_json::RawValue`. PostgreSQL receives the already
encoded values without constructing an intermediate JSON value tree. Database-only
serializers omit absent optional fields, reducing stored JSON without changing
decoded events or public metadata fields. Exhaustive field destructuring makes
new audit fields require an explicit serializer update. A temporary
terminal-to-role lookup preserves event ordinals and role iteration order.
Preparation occurs before the SQL transaction and remains within overall commit
and full-server-cycle timing.

Private observations, role projections and runtime state are each batched across
all games in the existing commit boundary. Game/journal parents precede audit
inserts; private rows precede role rows. Proposed role sequence heads publish only
after commit succeeds. Empty batches, unchanged runtime-row suppression, coverage,
retention floors, recovery and fail-closed behavior remain compatible. No migration,
index, canonical snapshot, public audit API or dependency-pin change is required.

The opt-in trace separates preparation, private inserts, role inserts, runtime
upserts, aggregate SQL and final commit, linked by process-local commit IDs.
Five-second database samples preserve checkpoint, WAL and I/O counters and reset
timestamps; optimized runs also sample aggregate client-backend wait events. Settings are recorded; durability and timing settings are not altered.
The harness accepts `--binary` to compare preserved executables across rebuilds.

## Measurement method

The comparison uses three sequential matrices: an instrumented baseline,
optimized volatile then PostgreSQL, and optimized PostgreSQL then volatile.
Each matrix contains regional (11 clients), global (4 clients) and mixed
(five games/50 clients) profiles with 60 seconds warmup, 300 seconds measurement,
seed 12787750 and reconnects every 60 seconds. These client counts intentionally
match the preceding audit checkpoint, including its limited global workload.
Retention is disabled in those profiles. A separate 75-second regional PostgreSQL
smoke uses one client and one-day retention.

Preserved baseline and optimized binary SHA-256 values and source-file fingerprints
identify the builds; metadata revision/dirty fields describe the checkout when
the harness starts, which can differ from a preserved executable's source.
The baseline binary is the prior implementation with added timing instrumentation.
Its preparation span covers cloning/projection; JSON parameter construction still
falls within aggregate SQL timing. The optimized preparation span includes JSON
encoding. Compare overall cycles/commits rather than treating those preparation
spans as identical work or adding nested span durations together.

Functional checks and builds overlapped the initial volatile baseline profiles;
those volatile numbers are descriptive and do not establish isolated causal
speedups. The regional/global PostgreSQL baselines run after those checks. Additional
encoding regression checks and a rebuild overlap the mixed PostgreSQL baseline;
its timing is descriptive. Both optimized matrices run without competing checks
or harnesses. Independent stateful profiles are not identical
event-stream replays; audit volumes and sampled resource growth must accompany
any timing comparison.

## Validation

Validation passes 215 fast workspace Rust tests, 14 PostgreSQL integration tests,
45 frontend tests, all 61 browser cases across the general suite and isolated
persistence run, and 16 harness tests including real-container interruption cleanup.
Formatting, strict workspace Clippy, Rust 1.89 compatibility and dependency-pin
verification pass. Production web startup JavaScript is 264.2 KiB against the
350 KiB budget; the existing large dynamic-bundle warning remains.

New fixtures cover sparse/full JSON round trips, shared terminals, private events, equal timestamps, empty
batches, JSON escaping/Unicode, multi-game atomic writes, pagination and filters,
and restart without audit duplication. Failures at private-row, role-row and
runtime-state writes roll back every game journal and leave proposed heads
unpublished. Retention protection, cleanup rollback and unchanged runtime tuple
versions remain covered. All four core benchmark projection hashes match the
preceding communications-audit checkpoint.

## Measurements and acceptance

All nineteen traces are complete, with zero client errors, failed durable
operations or dropped samples. Independent reanalysis reproduces saved timing,
audit, retention and resource records. Validity does not imply a budget pass.

| Profile | Baseline p95 / max ms | Optimized p95 / max ms | Reverse-order p95 / max ms |
| --- | ---: | ---: | ---: |
| regional-volatile | [10.1 / 35.7](2026-10-11-audit-write/baseline/regional-volatile.json) | [11.5 / 21.9](2026-10-11-audit-write/optimized/regional-volatile.json) | [10.8 / 14.8](2026-10-11-audit-write/optimized-reverse/regional-volatile.json) |
| global-volatile | [6.3 / 9.5](2026-10-11-audit-write/baseline/global-volatile.json) | [5.7 / 6.4](2026-10-11-audit-write/optimized/global-volatile.json) | [5.5 / 6.1](2026-10-11-audit-write/optimized-reverse/global-volatile.json) |
| mixed-volatile | [33.0 / 38.8](2026-10-11-audit-write/baseline/mixed-volatile.json) | [32.9 / 39.0](2026-10-11-audit-write/optimized/mixed-volatile.json) | [35.2 / 41.6](2026-10-11-audit-write/optimized-reverse/mixed-volatile.json) |
| regional-postgres | [165.7 / 2614.6](2026-10-11-audit-write/baseline/regional-postgres.json) | [150.0 / 2400.3](2026-10-11-audit-write/optimized/regional-postgres.json) | [134.4 / 163.4](2026-10-11-audit-write/optimized-reverse/regional-postgres.json) |
| global-postgres | [70.3 / 121.4](2026-10-11-audit-write/baseline/global-postgres.json) | [78.3 / 136.8](2026-10-11-audit-write/optimized/global-postgres.json) | [68.0 / 118.3](2026-10-11-audit-write/optimized-reverse/global-postgres.json) |
| mixed-postgres | [493.9 / 1627.8](2026-10-11-audit-write/baseline/mixed-postgres.json) | [430.1 / 517.7](2026-10-11-audit-write/optimized/mixed-postgres.json) | [419.4 / 494.4](2026-10-11-audit-write/optimized-reverse/mixed-postgres.json) |

Regional PostgreSQL p95 improves from 165.7 to 134.4–150.0 ms, within the
250 ms limit. The first optimized matrix **fails the 1,000 ms maximum-cycle
limit** at 2,400.3 ms; the reverse-order repetition passes at 163.4 ms.
**Regional durable performance acceptance remains open.** All sampled reconnects
are below 3,000 ms. Global/mixed timings remain descriptive without an invented
cycle budget. The mixed baseline overlaps checks as described above.

### Storage and resources

| PostgreSQL run | Audit observations | Encoded private metadata MiB | Database growth MiB | WAL growth MiB | RSS first → last / peak MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline/regional | 237,199 | 166.3 | 364.2 | 453.7 | 94.6 → 144.3 / 150.1 |
| baseline/global | 17 | 0.0 | 20.5 | 47.0 | 320.0 → 351.2 / 374.5 |
| baseline/mixed | 712,061 | 499.2 | 1157.7 | 1537.4 | 878.6 → 1232.2 / 1256.0 |
| optimized/regional | 237,199 | 153.6 | 339.8 | 430.9 | 112.6 → 150.5 / 159.6 |
| optimized/global | 17 | 0.0 | 20.9 | 47.4 | 348.9 → 404.2 / 404.2 |
| optimized/mixed | 711,640 | 461.0 | 1081.5 | 1470.9 | 1036.1 → 1209.4 / 1214.5 |
| optimized-reverse/regional | 237,199 | 153.6 | 347.8 | 444.6 | 175.0 → 179.1 / 203.9 |
| optimized-reverse/global | 19 | 0.0 | 20.5 | 48.1 | 318.5 → 327.9 / 328.0 |
| optimized-reverse/mixed | 714,972 | 463.5 | 1085.9 | 1478.6 | 1106.4 → 1181.6 / 1206.0 |

Growth uses first/last samples within each measured window. WAL measures bytes
generated, not retained disk size. Private metadata bytes count the encoded
capture arrays; role projections, indexes, journals and snapshots contribute
additional database/WAL work. Sparse optional-field encoding reduces the private
representation; it does not discard observations. Independent event streams and
sampling windows prevent a precise causal-overhead claim. The global profile
generates little audit traffic and is not a busy-audit acceptance workload.

### Remaining flush stalls

The [stage diagnostics](2026-10-11-audit-write/diagnostics.json) identify the
largest measured regional persistence commit at 2,382.4 ms: preparation takes
2.1 ms, aggregate SQL takes 80.9 ms, and final commit/flush takes 2,298.6 ms.
The baseline largest persistence commit similarly spends 2,487.7 of 2,595.6 ms
in final flush. Full server cycles also include processing outside persistence.

Nearby optimized samples observe the client backend waiting on PostgreSQL
`WalSync`, consistent with synchronous WAL-flush latency. Five-second samples,
their sampling duration and checkpoint counters do not establish an underlying
host/disk or checkpoint root cause. I/O timing remains disabled. A 1.74-second
commit in reverse-order warmup is preserved in the raw trace but excluded from
measured-window budgets. Its passing measured maximum does not erase the first
optimized run’s failure.

The [75-second retention smoke](2026-10-11-audit-write/retention-smoke.json)
is valid and exercises successful maintenance with no eligible/deleted records,
as expected for fresh data. It passes p95 at 207.8 ms but **fails maximum cycle**
at 2,080.7 ms. This separate finding reinforces that the flush-latency gap remains.
Aged deletion and multi-batch draining remain covered by PostgreSQL fixtures.

Follow-up work should investigate synchronous WAL/storage latency on this host
and repeat the regional budget checks with fixed durability settings. This
increment does not certify regional maximum-cycle acceptance or tune the database.

### Reproduction and artifacts

The host is WSL2 Linux x86_64, AMD Ryzen 7 9800X3D with 16 logical CPUs,
Rust 1.96.0 and Node 24.15.0. PostgreSQL is 17.11 from the same pinned image
`postgres@sha256:b0f9560a2de083e2cc7382e75f808c7381a32852a7ec49117deedb300e552b24`.
Fsync, synchronous commit and full-page writes remain enabled. The dependency
pin remains `2f13199e665806b5d0ac0f0f4db07eb436e0a008`.

[Core hashes](2026-10-11-audit-write/core.json),
[baseline fingerprint](2026-10-11-audit-write/baseline-fingerprint.json),
[optimized fingerprint](2026-10-11-audit-write/optimized-fingerprint.json), and
[independent reanalysis](2026-10-11-audit-write/reanalysis.json) accompany the
linked compact profiles. These uncommitted builds are based on
`256b9f1`; file and binary hashes identify the actual sources/executables.
Complete raw traces, summaries, metadata, preserved binaries, validation logs
and comparison/reanalysis scripts remain in
`tmp/performance/audit-write-20261011/` (ignored local artifacts). Existing
checkpoint artifacts are preserved.

Use [the short-run commands](LOAD_TESTING.md#postgresql-comparison-measurement-version-2)
with explicit clients 11/4/50, 60-second warmup, 300-second measurement,
reconnects every 60 seconds, and the pinned image. `--binary` selects the preserved
build. Choose fresh output directories. Budget-failing runs remain valid findings;
do not omit them or present a favorable repetition as full acceptance.

No long soak was run. Browser/GPU timing, delta bandwidth, roadmap-scale loads,
historical replay and long-term memory/database stability remain separate work.
Default indefinite archive retention permits storage growth.
