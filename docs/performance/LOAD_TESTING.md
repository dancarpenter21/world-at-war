# Full-server performance measurements

The integrated performance increment measures the current Regional Joint Campaign
and Global Crisis scenarios. It does not certify 1,000 platforms, 10,000 background
objects/tracks, one game with 50 seats, browser rendering, delta delivery, or 20 Hz
updates. Current clients receive full snapshots at approximately 1 Hz.

## Reproduce

Use Linux/WSL, Node 22+, the pinned sibling c3mesh checkout, and a release server.
Run from a clean checkout without competing builds or other benchmarks:

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

`--clients N` fills additional games using the profile's seat distribution
(up to 200 clients). `--seed`, `--warmup`, `--duration`, and `--reconnect-every`
are configurable. Defaults are mixed/50 clients, seed 12787750, 60 seconds of
warmup, 300 measured seconds, and reconnects every 300 seconds. CI uses a
120-second run with reconnects every 30 seconds. `npm run perf:server -- ...`
from `web/` is an equivalent entry point; relative output paths follow the caller.

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
outage. The five-minute reference profiles normally contain no reconnect cycle;
the short CI run and one-hour soak exercise it. A run fails on unexpected HTTP or
WebSocket errors, forbidden reads succeeding, stalled tick/stream progress for
15 seconds, interrupted execution, or incomplete telemetry.

## Artifacts and interpretation

- `metadata.json`: revision, dirty status, binary hash, dependency pin, hardware,
  toolchain, scenario/catalog identifiers, seed, client assignments and time window.
- `server.jsonl`: bounded-channel local trace of complete ticks, scheduling delay,
  shared game-lock waits/holds by source location, and state/stream projection costs.
- `clients.jsonl`: request latency/status/size, stream gaps, command outcomes,
  reconnects, generator scheduling lag and five-second resource samples.
- `summary.json`: measured-window histograms by game, client and operation, budget
  comparisons, resource series and run validity. `server.log` records process output.

Server tick timing includes AI, authority, simulation and existing audit writes.
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
