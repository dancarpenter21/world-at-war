# Full-server performance measurements

The integrated performance increment measures the current Regional Joint Campaign
and Global Crisis scenarios. It does not certify 1,000 platforms, 10,000 background
objects/tracks, one game with 50 seats, browser rendering, delta delivery, or 20 Hz
updates. Current clients receive full snapshots at approximately 1 Hz.

## Reproduce

Use Linux/WSL, Node 22+, the pinned sibling c3mesh checkout, and a release server.
Run from a clean checkout without competing builds or other benchmarks:

Regional and mixed full-server profiles now require an installed terrain dataset.
Set `TERRAIN_DIR` to its absolute directory; see
[terrain setup and model limits](../SENSING_IMPLEMENTATION.md).
The harness forwards that path to its isolated server and records the manifest and
checksum in `metadata.json`. Synthetic terrain is suitable for labelled harness
validation only; it does not establish real-terrain reference acceptance. The
ordinary core benchmark retains its legacy workload; add `--sensing` for the new
model and supply terrain.

CI creates a fresh, explicitly synthetic flat dataset with
`python3 scripts/setup-ci-terrain.py --output <fresh-directory>` for the short
performance and production-container smoke checks. It never downloads terrain
or replaces an existing dataset; these runs validate the harness and runtime.

```sh
sh scripts/setup-c3mesh.sh --verify-only
cargo build --release --locked -p world-at-war-server
npm ci --prefix web
npm run test:performance --prefix web
mkdir -p tmp/performance

target/release/world-at-war-server --benchmark \
  --warmup 30 --ticks 120 --repeats 2 --seed 12787750 \
  > tmp/performance/core.json

node web/scripts/performance-server.mjs --profile regional \
  --warmup 60 --duration 300 --output tmp/performance/regional
node web/scripts/performance-server.mjs --profile global \
  --warmup 60 --duration 300 --output tmp/performance/global
node web/scripts/performance-server.mjs --profile mixed \
  --warmup 60 --duration 300 --output tmp/performance/mixed
node web/scripts/performance-server.mjs --profile mixed \
  --warmup 60 --duration 3600 --output tmp/performance/soak
```

Output directories must not already exist. The harness launches and stops its own
release server, uses a temporary runtime directory, and runs a local catalog
provider with two fixture objects. It does not read `.env` or use real catalog
credentials. Existing development services are not restarted. The release binary
must be rebuilt after Rust changes; its SHA-256 is recorded with repository state.

`--binary /absolute/path/to/server` selects a preserved release binary for a
before/after comparison; its SHA-256 is recorded instead of the default binary.
The repository revision and dirty flag describe the current checkout, so retain
the matching source fingerprint separately when using a preserved binary.

`--clients N` fills additional games using the profile's seat distribution
(up to 200 clients). `--seed`, `--warmup`, `--duration`, and `--reconnect-every`
are configurable. Defaults are mixed/50 clients, seed 12787750, 60 seconds of
warmup, 300 measured seconds, and reconnects every 300 seconds. CI uses a
120-second run with reconnects every 30 seconds. `npm run perf:server -- ...`
from `web/` is an equivalent entry point; relative output paths follow the caller.

`--time-scale` selects 0.25, 1, 2, 5, 10, 30 or 60 (default 1) through each
host's authenticated control endpoint before starting. Metadata records the
requested speed and 60 Hz target. One host per game additionally polls the game
summary once per second to check fractional/frame progress, selected speed and
running status. These requests are included in client load and latency telemetry.
Compare runs using the same measurement version and request workload.
The `games` metadata entries retain creation-time summaries; use top-level
`requested_time_scale` and committed-frame progress for the measured speed.

## Workload

| Profile | Games | Distinct guest players |
| --- | --- | --- |
| Regional | One regional | Nine operational roles, two granted observers |
| Global | One global | Thirteen operational roles |
| Mixed | Three regional, two global | 11 + 11 + 11 + 13 + 4 = 50 |

Each guest uses its own cookie jar, CSRF token and real role lease. Hosts claim
operational roles; observers need explicit host grants. Setup checks that another
player's role and an invalid lease generation cannot be read. Roles renew every
30 seconds. Client starts are staggered by 20 ms.

Operational clients poll state and planning once per second after the previous
request completes, poll authority every two seconds, and consume the network
WebSocket. Observers consume the truth stream. The first player in each game is
a pilot or flight lead and submits a zero-velocity movement intent every 30
seconds. Receipt polling records modeled outcomes, including delivery failures.
The workload measures the existing authority and audit paths without disabling AI.

Every five minutes a rotating 10% of players stops requests and closes sockets
for ten seconds. Recovery time starts when connectivity returns, includes lease
resume, a fresh HTTP snapshot and WebSocket reopening, and excludes the intended
outage. Operational HTTP snapshots verify fractional frame advancement when
available. Truth/historical snapshots may retain the same whole-second tick;
they must not regress, and the independent host-clock watchdog verifies progress.
Planned host outages suspend that game’s clock probe deadline, which resets after
the WebSocket handshake completes; committed-frame throughput still includes
the entire outage window. Clients stay offline throughout recovery, so a partially
opened connection cannot expose the old clock deadline. The five-minute reference profiles normally contain no reconnect cycle;
the short CI run and one-hour soak exercise it. A run fails on unexpected HTTP or
WebSocket errors, forbidden reads succeeding, stalled committed-clock progress for
15 seconds, an expected operational whole-second stream update remaining
undelivered for 15 seconds, an observer stream silent for 15 seconds, interrupted
execution, or incomplete telemetry. A newer expected tick does not extend an
outstanding delivery deadline; receiving its required tick clears that deadline.

## Artifacts and interpretation

- `metadata.json`: revision, dirty status, binary hash, dependency pin, hardware,
  toolchain, scenario/catalog identifiers, seed, client assignments and time window.
- `server.jsonl`: bounded-channel local trace of complete ticks, scheduling delay,
  shared game-lock waits/holds by source location, and state/stream projection costs.
- `clients.jsonl`: request latency/status/size, stream gaps, command outcomes,
  reconnects, generator scheduling lag and five-second resource samples.
- `summary.json`: measured-window histograms by game, client and operation, budget
  comparisons, resource series and run validity. `server.log` records process output.

Per-game tick timing includes AI, authority, simulation and existing file audit writes,
but excludes PostgreSQL persistence. See measurement version 2 below.
Scheduling delay and lock contention are separate metrics; a short tick does not
prove timely service. HTTP projection build timing excludes lock acquisition;
operational WebSocket build timing explicitly includes it. Observer projection
build and serialization are measured separately under the game lock. HTTP body
sizes measured by the harness are decoded unless an encoded Content-Length is
available. WebSocket byte counts exclude framing/TCP overhead. This is not a
wire-level bandwidth measurement or a delta-size measurement.

Histograms use 0.1 ms timing bins (rounded upward) up to ten seconds, with an
overflow bin reported conservatively as the exact maximum. Byte metrics use
one-byte bins up to ten million bytes. Mean/max remain exact. Throughput uses
active one-second buckets; derive whole-run average rates from sample counts and
the recorded duration, including deliberate outages. CPU samples are cumulative
process CPU seconds, not utilization percentages; resource series distinguish the
server and load generator. Memory and audit growth have no invented pass threshold.

Normal completion flushes the server writer and records written/dropped counts.
A missing footer or any dropped samples invalidates the run. The writer uses an
8,192-record channel; clients also reject a trace backlog over 8 MiB. Metrics never
contain request bodies, projection payloads, cookies or credentials. Raw traces
are ignored local output or CI artifacts, not game persistence or durable audit.

The summary evaluates regional full-server tick p95 <=250 ms, max <=1,000 ms,
and reconnect <=3 seconds. Budget misses are findings by default. Add
`--check-budgets` to make them failures on reference hardware. CI gates harness
correctness, not timing on shared runners. A short run without reconnect samples
does not certify reconnect latency. Optimization, the full roadmap scale and the
24-hour release soak remain separate increments.

## Committed frames (measurement version 4)

A successful `server_cycle` now carries a process-local `cycle_id` and per-game
`frame_progress`: actual frame/game-time deltas, whole-second boundaries, final
clock values and requested speed. Progress publishes only after the entire cycle
commits, including PostgreSQL persistence or volatile audit flush. Failed cycles
have no committed progress; maintenance-only cycles do not count as frames.
`persistence_commit.cycle_id` links shared transaction stages to the cycle via
`commit_id`. Private `tick` records remain pre-persistence processing measurements;
`whole_seconds` separates fractional motion work from game-second boundary work.

`summary.json.frames` reports each game's committed frames divided by the entire
measured wall-time window, achieved game-time rate, requested-speed percentage,
interval percentiles and intervals exceeding 1/60 second. Per-second series include
zero-progress buckets; a partial final bucket uses its actual duration. The window
is start-inclusive/end-exclusive. Intervals count only when both commits fall in
the window. Rates count frame deltas at completion, so a frame straddling a window
edge belongs to the window containing its completion. The 60 Hz value is a target,
not a new pass tolerance. A valid slow run is still useful baseline evidence.

`persistence` adds successful commit/boundary/checkpoint counts and journal/submitted
snapshot byte totals per measured wall second and aggregate committed game second.
These are encoded/submitted bytes, not on-disk growth. `database_growth` measures
the difference between first and last in-window database/WAL samples and records
that shorter sampling interval; it is not exact growth over the full window.
`slowest_cycles` retains the twenty largest advancing cycles and their correlated
commit/flush stages. Shared transaction costs remain server-wide. Failed operations,
missing committed progress, incomplete windows and dropped traces invalidate runs.
Historical traces remain analyzable; older records cannot certify frame throughput.

The role-scoped diagnostics API adds `target_frame_hz`, `committed_frame_hz`,
`committed_frame_interval_ms`, `committed_frame_overruns` and
`last_committed_frame_age_ms`. `achieved_time_scale` now uses committed progress.
Rates use up to 120 committed intervals plus time since the latest commit, becoming
unavailable after three seconds without progress. Pause/resume and speed changes
reset rate samples. These observational fields never enter snapshots or gameplay
projections. The browser panel labels persistence-inclusive rates separately from
frame processing time and marks paused/stale/waiting states.

For short reference measurements, run each profile in each storage mode separately
at 1× with 60-second warmup, 300-second measurement and 60-second reconnects, then
repeat in reverse storage order. Brief speed checks use one global game in each
mode with 15-second warmup and 45-second measurement. Synthetic terrain results
must remain explicitly labelled. Neither procedure authorizes a long soak.

## Retention measurements

The opt-in server trace emits aggregate `retention` records every five seconds,
including paused games. They contain c3mesh device, interference/mobility history,
interference-contribution, pending-event/reception counts and the retained-time
boundary; application transport messages/fragments/completed IDs; and server
message/history, authority, receipt and pending-delivery counts. No message bodies,
identities or credentials are included beyond the existing game identifier.

`summary.json.retention` groups first/last/peak counts and the sample series by
game. Older traces remain readable and return an empty retention object. Payload
byte counts measure stored byte-vector lengths, not container overhead, spare
capacity, complete message object sizes or exact heap allocation.

Compaction runs at completed network boundaries and keeps every snapshot needed
by the oldest pending reception, an anchor at or before that time, and future
changes. A long reception intentionally retains its entire interference interval.
When no reception or future update remains, each active device retains one
interference and one mobility anchor. Compare these structural bounds and the
RSS series with the baseline; RSS alone cannot distinguish retained allocations
from allocator high-water marks. Player-visible message history, completed IDs,
intent receipts and audit files still grow with activity and are not expired by
this increment. Their archival policy belongs to persistence work.

## PostgreSQL comparison (measurement version 2)

Pass `--storage postgres` to measure durable operation. The default remains
`--storage volatile`. PostgreSQL mode requires Docker access and starts its own
PostgreSQL 17 container with a fresh named disk volume and loopback-only random
port. It never uses the development database or reads `.env`. Credentials are
generated per run and excluded from artifacts. Normal cleanup and handled
interruptions remove that container and volume; cleanup failures invalidate the
run. A forcibly killed harness may leave its named resources behind: metadata
records their names for targeted cleanup, never use a system-wide prune.

`--postgres-image postgres@sha256:...` selects an exact image for comparisons.
The default is `postgres:17-alpine`; metadata records the resolved image ID,
repository digest, PostgreSQL version and durability settings. Use the same image,
hardware, toolchain and sampling settings for before/after runs. Fsync,
synchronous commit and full-page writes must remain enabled.

```sh
node web/scripts/performance-server.mjs --storage postgres --profile regional \
  --warmup 60 --duration 300 --reconnect-every 60 \
  --output tmp/performance/durable-regional
```

Run regional, global and mixed profiles sequentially in each storage mode. A
reference comparison uses 60 seconds warmup, 300 measured seconds and reconnects
every 60 seconds; record actual reconnect coverage rather than claiming every
player reconnected. Repeat the final matrix in reverse storage-mode order.
These are separate short runs, not authorization to run a long soak.

Version 2 adds `server_cycle` timing from acquisition of the shared games lock
through backups, lease expiry, all game advances, boundary encoding and hashing,
SQL commit and post-commit bookkeeping. It excludes gate/lock waits; the separate
scheduled-to-committed metric includes them. Shared transaction costs belong to
the whole cycle, not individual games. Idle cycles do not dilute percentiles.
The older `tick` timing and diagnostics API still measure game processing before
persistence; they cannot certify durable tick performance.

Additional local spans measure encoding, hashing, mutation boundaries, durability
gate waits (HTTP, simulation and streams), storage-lock waits, SQL execution and
transaction commit. These spans nest: their durations must not be summed as if
independent. Cancelled/failed durable boundaries are not successful cycles.
Aggregate counts include journal bytes, submitted snapshot bytes, checkpoints and
archived events. No message or snapshot bodies are recorded. Communications audit capture adds
aggregate event and metadata-byte counts. Retention samples include audit buffer
and packet-correlation counts. Cleanup spans/results report duration, deleted
rows, failures, eligible backlog and oldest eligible age; the analyzer preserves
these in `summary.json.audit` and timing metrics. The short harness's
`--archive-retention-days N` configures and records the opt-in PostgreSQL policy;
the default is 0.

Audit writes also emit `persistence_audit_prepare`, `persistence_audit_records`,
`persistence_audit_roles`, and `persistence_audit_runtime` spans. A process-local
`commit_id` associates them with SQL, final-flush and overall commit spans.
Preparation includes projection and JSON encoding before the transaction; its
cost remains inside the overall commit and server-cycle timing. Optimized spans
record private/role/runtime row and encoded byte counts. Private `audit_bytes`
counts the encoded private rows as compact JSON arrays, including punctuation;
role/runtime bytes are separate. Sparse storage omits absent optional fields,
so compare byte counts as encoded representation sizes, not packet payload sizes. These overlapping spans must not be summed.

Five-second PostgreSQL samples additionally preserve `pg_stat_checkpointer`,
`pg_stat_wal`, and `pg_stat_io` counters, including statistics-reset timestamps.
Optimized runs also record aggregate client-backend wait-event counts without
query text, connection identities or credentials. These are instantaneous samples,
so a missing wait observation does not exclude a stall between samples.
Recorded settings include WAL/checkpoint sizes and I/O timing switches. Sampling
does not enable timing, reset counters, change checkpoints, or relax durability.
Compare cumulative counters only across samples with unchanged reset timestamps;
zero timing counters with timing disabled do not prove an absence of I/O waits.
Sample-level coincidence with a commit stall is correlation, not a root-cause
proof. Legacy traces without these fields remain readable.

Five-second database samples contain database/table/index sizes, WAL growth since
container readiness, and Docker container CPU/memory/block-I/O measurements.
Docker memory includes the container's processes; it is not server-process RSS.
Sampling duration is reported. All growth claims use measured-window samples;
WAL includes the workload and PostgreSQL background activity in this isolated
instance. These metrics have no invented memory or storage pass threshold.

A new run must complete its measurement window and contain committed progress for
every game, a complete trace footer, and (for PostgreSQL) commit and resource
samples. Failed durable operations invalidate the run. Legacy traces remain
readable but cannot satisfy the new validation contract. The standalone regional
budget now uses the complete server cycle (p95 <=250 ms, maximum <=1,000 ms).
Mixed/global full cycles are reported without assigning shared commit costs to
regional games. Reconnect remains <=3 seconds. CI gates validity rather than
machine-dependent timing.

`PERFORMANCE_POSTGRES_TESTS=1 npm run test:performance --prefix web` additionally
checks real-container interruption cleanup. The ignored Rust persistence tests
include an injected database delay and failure to verify that full-cycle timing
includes commits and never reports a rolled-back tick as committed.

### Monotonic harness clock (measurement version 3)

Client timestamps, deadlines, stream gaps, reconnect scheduling and measurement
windows advance from a single epoch anchor using the monotonic process clock.
WSL wall-clock adjustments previously produced negative gaps in several version 2
reference attempts; those runs remain invalid evidence and are not repaired into
acceptance results. Version 3 preserves ordinary epoch-shaped timestamps for
trace analysis while making elapsed intervals independent of wall corrections.

## Unattended soak runner

`performance-soak.mjs` runs independently of an agent. By default it runs mixed
50-client loads sequentially: one measured hour in volatile mode, then one in
fresh Docker PostgreSQL, each with 60 seconds warmup. Total runtime is a little
over two hours. It uses the same harness, measurements and cleanup as short runs.
Prepare the dependency, release server and npm dependencies using the commands
above, start Docker Desktop, then run from the repository root:

```sh
node web/scripts/performance-soak.mjs --output tmp/performance/soak-reference
```

To keep it running after closing the terminal on Linux/WSL:

```sh
mkdir -p tmp/performance
nohup node web/scripts/performance-soak.mjs \
  --output tmp/performance/soak-reference \
  > tmp/performance/soak-launch.log 2>&1 < /dev/null &
echo $! > tmp/performance/soak-runner.pid
```

Keep the computer awake and Docker/WSL running; `nohup` survives terminal closure,
not a machine restart or WSL shutdown. Choose a new output directory for each run.
For a single 24-hour durable run, add `--storage postgres --duration 86400`.
Use `--postgres-image postgres@sha256:...` to pin the image for comparisons.
`--check-budgets` makes applicable timing misses fail the run; otherwise they are
reported findings. `--help` lists profile, player-count and scheduling options.
`npm run perf:soak --prefix web -- ...` is also available; npm runs from `web/`,
so supply an absolute output path or account for that directory. The runner does not rebuild binaries or install dependencies.

Afterward, examine `report.md` for outcomes, cycle timing, resource first/last/peak
values and errors. `report.json` preserves aggregate resource and retention series;
each mode’s directory contains the complete metadata, raw traces and summary.
`runner.log` records starts, minute heartbeats and completion; `volatile.log` and
`postgres.log` contain harness progress. `status.json` is atomically updated at
mode boundaries and final completion. A final report is written even when a mode
fails; the other independent mode still runs. There are no automatic retries.
The runner exits zero only when all requested runs are valid and completed.

Stop gracefully with Ctrl-C in the foreground or `kill -TERM` using the saved
runner PID in the background. It forwards termination and waits for cleanup.
Each mode has a watchdog of warmup + duration + ten minutes (configurable via
`--timeout-grace`), followed by up to two minutes for cleanup before forced exit.
Timeouts and interruptions fail the run. Forced exit, power loss or shutdown may
leave named Docker resources; use the affected run’s metadata for targeted cleanup.
Missing final reports or a remaining `running` status indicate incomplete execution.
Neither passing nor a quiet log establishes bounded memory or full-roadmap scale;
inspect retention, RSS/storage growth and actual reconnect coverage in the results.
