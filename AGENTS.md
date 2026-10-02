# Repository Guidelines

## Project Structure & Module Organization

This repository implements a low-fidelity war simulator with a Rust simulation core, a Cesium/React web client, communications catalogs, and Docker-based local runtime. Keep the layout predictable:

- `crates/` or `src/`: Rust entity-component simulation, line-of-sight, sensors, movement, communications, and authority logic.
- `web/`: Cesium map client and browser-facing assets.
- `tests/`: integration tests and scenario-level fixtures.
- `assets/` or `data/`: static catalogs such as airport, platform, orbital, terrain, and scenario data.
- `docker-compose.yml`: local multi-service development and test environment.

Keep domain data separate from executable code so simulation logic can be tested with small fixtures.

## Build, Test, and Development Commands

Rust builds require Rust 1.89+ and the sibling `../c3mesh` checkout pinned in `c3mesh-revision.txt`. Run `scripts/setup-c3mesh.ps1` (or `sh scripts/setup-c3mesh.sh`) before a fresh build; use its verify mode for reproducible validation.

- `cargo fmt --check`: check Rust formatting.
- `cargo check --workspace --locked`: validate Rust compilation.
- `cargo test --workspace`: run Rust unit and integration tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: run Rust lint checks.
- From `web/`, `npm ci` and `npm run dev`: install locked dependencies and run the Cesium client.
- From `web/`, `npm test` and `npm run build`: run frontend unit tests, typecheck, and build.
- From `web/`, `npx playwright install chromium`, then `npm run test:e2e:network` or `npm run test:e2e:session`: run standalone browser tests without the Rust server.
- From `web/`, `npm run test:e2e:gameplay`: build the Rust server and run the real gameplay/transport regression with isolated local providers.
- From `web/`, `npm run test:e2e`: build the Rust server and run all browser integration tests.
- `docker compose up --build`: start the full local stack.
- `docker compose run --rm <service> <command>`: run service-scoped tests or maintenance.

## Coding Style & Naming Conventions

Use Rust 2021 idioms for simulation code. Format Rust with `cargo fmt` and lint with `cargo clippy`. Prefer clear domain names such as `SensorContact`, `AuthorityChain`, `LineOfSight`, and `CommunicationLink`.

For frontend code, use TypeScript where possible, keep components focused, and name files by feature, for example `MapViewport.tsx` or `unit-layer.ts`.

## Testing Guidelines

Place fast unit tests near the code they exercise and broader scenario tests under `tests/`. Favor deterministic fixtures for sensors, orbital tracks, and communications failures. Name tests after observable behavior, for example `detects_unit_when_inside_radar_horizon`.

For data-driven tests, include small fixtures rather than full external catalogs.

## Commit & Pull Request Guidelines

Git history is available in this workspace, so use verbose git commits and messages.

Pull requests should include purpose, test results, linked issues when applicable, and screenshots or clips for map-client changes. Note new data sources or external service dependencies.

## Security & Configuration Tips

Do not commit credentials for Spacetrack, catalog providers, or map services. Keep secrets in ignored local environment files, and document variable names without real values. Prefer .env with real values (not checked into source control) and .env.example files checked in but holding placeholder values.
