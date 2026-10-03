# Repository Guidelines

## Project Structure & Module Organization

This repository contains a Rust simulation/server workspace, a Cesium/React web client, scenario fixtures, and Docker-based local runtime. `README.md` describes implemented behavior; `IMPLEMENTATION_PLAN.md` distinguishes the current checkpoint from the broader target. Keep the layout predictable:

- `crates/` or `src/`: Rust entity-component simulation, line-of-sight, sensors, movement, communications, and authority logic.
- `web/`: Cesium map client and browser-facing assets.
- `tests/`: integration tests and scenario-level fixtures.
- `assets/` or `data/`: static catalogs such as airport, platform, orbital, terrain, and scenario data.
- `docker-compose.yml`: local multi-service development and test environment.

Keep domain data separate from executable code so simulation logic can be tested with small fixtures.

## Build, Test, and Development Commands

Run Rust commands at the repository root and npm commands from `web/`. The workspace requires Rust 1.89+ and the sibling `../c3mesh` checkout pinned by `c3mesh-revision.txt` (currently v0.2.0). Use `sh scripts/setup-c3mesh.sh` or `scripts/setup-c3mesh.ps1` for setup and their verify modes to check the pin. Commands:

- `cargo test`: run Rust unit and integration tests.
- `cargo check`: validate Rust compilation quickly during development.
- `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`: formatting and lint checks.
- `npm test` and `npm run build`: frontend tests, TypeScript checking and production build.
- `npm run test:e2e`: browser tests with Playwright Chromium; avoid live Space-Track test variables unless explicitly testing the provider.
- `npm install` and `npm run dev` from `web/`: install and run the Cesium client.
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
