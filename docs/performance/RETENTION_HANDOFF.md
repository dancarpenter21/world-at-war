# Memory retention implementation handoff — 2026-10-09

Work was committed and pushed for continuation at home. The implementation is
complete; post-change reference measurements and registry publication remain open.

## Branches and dependency

- World At War: `codex/memory-retention`. Implementation commit `5b05ca2`;
  container-input fix `2ff6116`.
- c3mesh: `codex/bounded-history`, commit
  `f1f716c42c0f5cde2e16eb64e6350e4313b76a7d`. This is the exact revision in
  `c3mesh-revision.txt`; it prepares version 0.4.0 but is **not published**.
- Both branches are on GitHub. No changes have been merged into either main branch.
  Registry credentials were unavailable locally; publication was deferred to home.

Fetch and check out the World At War branch, then run
`sh scripts/setup-c3mesh.sh` from its root. This selects the matching sibling
source revision; `sh scripts/setup-c3mesh.sh --verify-only` verifies it.

## Implemented and validated

- c3mesh explicitly compacts interference and mobility histories using the oldest
  pending reception start/current time as the boundary. It retains anchors and
  future changes; historical queries before the boundary return an explicit error.
- World At War compacts at completed network boundaries, including IFTU and
  post-mission reporting. Public history, completed-message IDs, intent receipts,
  authority records and audit files are preserved.
- Opt-in five-second retention telemetry reports aggregate counts and stored
  payload lengths. The analyzer retains first/last/peak counters and sample series
  and accepts legacy traces without retention records.
- The original one-hour trace was reanalyzed: all metrics match, with no missing
  records or client errors. Its RSS growth from 158.09 to 2,603.69 MiB remains the
  **before** baseline, not evidence that the new implementation solved it.
- 201 workspace Rust tests pass on stable and Rust 1.89; strict Clippy and formatting
  pass. 45 frontend tests, the production web build and five harness tests pass.
- c3mesh passes 66 all-feature tests and 61 no-default-feature tests, strict Clippy,
  formatting, Rust 1.85 tests and clean package verification/publish dry run.
  [All five c3mesh CI jobs passed](https://github.com/dancarpenter21/c3mesh/actions/runs/37979168535).
- Regional/Global preflight benchmarks matched all baseline projection hashes
  across two repeats. Those timings overlapped browser work and are not accepted
  as reference performance results.
- Container CI exposed missing compile-time benchmark inputs in the existing
  Dockerfile. Commit `2ff6116` copies the dependency pin and communications catalog
  into production/development build stages. Rebuild verification is tracked in
  [production-container CI](https://github.com/dancarpenter21/world-at-war/actions/runs/37979752984).

## Remaining measurements

Run without competing builds, browser suites or other benchmarks. Use the same
reference hardware for a direct before/after timing comparison; if using different
hardware at home, explicitly record that limitation. Each command creates a new
output directory and launches/stops only its own isolated server. Choose a fresh
parent directory if these output names already exist.

```sh
sh scripts/setup-c3mesh.sh --verify-only
cargo build --release --locked -p world-at-war-server
npm ci --prefix web
npm run test:performance --prefix web
mkdir -p tmp/performance/retention-reference
target/release/world-at-war-server --benchmark \
  --warmup 30 --ticks 120 --repeats 2 --seed 12787750 \
  --baseline docs/performance/2026-10-09/core.json \
  > tmp/performance/retention-reference/core.json
node web/scripts/performance-server.mjs --profile regional \
  --warmup 60 --duration 300 --check-budgets \
  --output tmp/performance/retention-reference/regional
node web/scripts/performance-server.mjs --profile global \
  --warmup 60 --duration 300 --check-budgets \
  --output tmp/performance/retention-reference/global
node web/scripts/performance-server.mjs --profile mixed \
  --warmup 60 --duration 300 --check-budgets \
  --output tmp/performance/retention-reference/mixed
node web/scripts/performance-server.mjs --profile mixed \
  --warmup 60 --duration 3600 --check-budgets \
  --output tmp/performance/retention-reference/soak
```

Check complete traces, zero client errors/dropped samples, tick/reconnect budgets,
and `summary.json.retention`. Drained networks with no future updates should have
one mobility and interference anchor per active device. Pending receptions can
legitimately retain more history. Publish metadata and summaries with before/after
RSS growth and the remaining application-history counts; do not claim constant
total memory or 24-hour stability. Update the implementation checkpoint and
performance documentation after these results exist.

## Release and CI follow-through

1. Inspect the latest branch runs for
   [project checks](https://github.com/dancarpenter21/world-at-war/actions/workflows/checks.yml),
   [containers](https://github.com/dancarpenter21/world-at-war/actions/workflows/containers.yml),
   and the [performance harness](https://github.com/dancarpenter21/world-at-war/actions/workflows/performance.yml).
2. Publish c3mesh 0.4.0 from the tested clean source with `cargo publish --locked`
   once Cargo credentials are configured locally. Do not paste credentials into
   issues, commits or this handoff. The public error enum gains a variant, so the
   0.4.0 version bump is intentional.
3. Record successful publication and remove the "prepared/unpublished" wording
   from README, AGENTS and the implementation checkpoint. Preserve the tested
   source pin, or rerun pin/build checks if choosing a subsequent merge commit.
4. Review/merge the branches using the final measurements. Durable accounts,
   persistence, recovery/replay and application-history archival remain separate work.
