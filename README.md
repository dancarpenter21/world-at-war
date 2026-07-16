# World At War

World At War is a server-authoritative, low-fidelity war-simulation prototype. It combines a Rust entity-component simulation, a Cesium/React operational map, a public Space-Track orbital catalog, and an authority workflow for command decisions.

The current implementation ships one **Global Crisis** scenario with 64 authored entities, thirteen claimable Blue command roles, a basic Red AI, White House and Pentagon command nodes, and a pinned public orbital snapshot. Authored entities and uncertain tracks use MIL-STD-2525D symbols.

The broader target architecture, planned simulation fidelity, and acceptance criteria are in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). Features described there are not necessarily implemented yet.

## What is implemented

- A deterministic Rust ECS simulation with one-second ticks, platform movement, server-side projections, and simple Red patrol AI.
- A lobby that creates and joins games, role claiming, game start/pause controls, and REST/WebSocket state delivery.
- A Cesium operational map that keeps authored owned units and uncertain tracks visually separate from the public orbital catalog, reconciling entities in place so movement ticks do not recreate or flicker MIL-STD-2525D icons.
- A lazy full-screen space-asset workspace with worker-based bulk propagation, point-primitive rendering, UTC playback, search/facets, sourced payload cards, and authority-routed satellite requests.
- A versioned authority definition: roles, operational/support/advisory/transmit relationships, policies, direct grants, approval sequences, vacant-role resolution, and human approval or denial of requests.
- A Space-Track GP catalog integration with encrypted remembered credentials, cached snapshots, clear diagnostics for credential/access/service failures, and per-game catalog pinning.
- A global airport/runway cache using public-domain OurAirports data with an authoritative FAA NASR overlay for U.S. facilities, declared distances, pavement ratings, and reported gross-weight limits.
- A Docker Compose edge proxy that serves the web client and routes `/health`, `/v1/`, and WebSocket traffic to the Rust server.

Current limitations: communications are an always-reachable gate, sensor and track behavior is intentionally simplified, and the broader platform, terrain, logistics, cyber, and multi-source catalog systems remain planned work.

## Prerequisites

- Rust toolchain compatible with the Rust 2021 workspace.
- Node.js 22+ and npm for frontend development.
- Docker Compose v2 for the container workflows.
- A Space-Track account only when creating a scenario that requires the public orbital catalog.

## Run locally

Run the server from the repository root:

```sh
cargo run --package world-at-war-server
```

In a second terminal, run the web client:

```sh
cd web
npm ci
npm run dev
```

Open `http://localhost:5173`. The Vite development server proxies API calls to `http://localhost:8000` by default. Set `VITE_API_BASE` in the frontend environment (for example, `VITE_API_BASE=https://api.example.test npm run dev`) only when the browser must use a different API origin.

## Run with Docker

The default Compose file is production-shaped: it builds a release Rust server and static web assets, then exposes only the Nginx edge proxy.

```sh
docker compose up --build
```

Open `http://localhost:8080`, or the port set by `APP_PORT`. The internal server and web containers are not published directly.

For containerized development with hot reloading, use the development override:

```sh
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

Open the same edge-proxy URL. Source directories are bind-mounted; `cargo watch` polls for Rust changes and restarts the server, while Vite uses polling and HMR for frontend changes. Named volumes preserve Cargo artifacts/downloads, frontend dependencies, and the Space-Track cache between restarts. Stop either stack with the matching `docker compose ... down` command.

Source edits are picked up without rebuilding images, and Cargo manifest changes are handled by `cargo watch`. After changing `web/package.json` or `web/package-lock.json`, refresh the named dependency volume with `docker compose -f docker-compose.yml -f docker-compose.dev.yml run --rm web npm ci`, then restart the web service. Rebuild the development images after changing either Dockerfile or its toolchain.

## Configuration and Space-Track

Copy [`.env.example`](.env.example) to an ignored root `.env` file for Compose configuration. Direct server runs read their configuration from the shell environment. `VITE_API_BASE` is a frontend build/development variable, so set it in the frontend environment rather than relying on the root Compose file.

| Variable | Purpose |
| --- | --- |
| `APP_PORT` | Host port for the Compose edge proxy; defaults to `8080`. |
| `BIND_ADDR` | Rust server bind address for direct runs; defaults to `0.0.0.0:8000`. Compose fixes this to the internal server address. |
| `VITE_API_BASE` | Optional browser API-origin override for direct Vite runs or custom frontend builds. |
| `SPACETRACK_USERNAME` / `SPACETRACK_PASSWORD` | Optional server-side Space-Track credentials, instead of entering them in the UI. Never commit real values. |
| `ADMIN_SETUP_TOKEN` | Optional bearer token required to configure catalog credentials through the UI. |
| `COOKIE_SECURE` | Set to `true` or `1` only when HTTPS terminates in front of the application. |
| `HOST_UID` / `HOST_GID` | Optional local user/group IDs for the Compose server process; defaults to `1000:1000` so it can read and update the bind-mounted catalog cache. |
| `SPACE_CARDS_DIR` | Optional path to offline-generated satellite cards; defaults to `data/generated/space-cards`. |
| `AIRPORT_CACHE_DIR` | Airport raw-source and normalized snapshot cache; defaults to `data/cache/airports`. |
| `AIRPORT_REFRESH_MAX_AGE_SECONDS` | Age after which startup schedules a background airport refresh; defaults to `86400`. |
| `FAA_NASR_APT_URL` | Optional URL pin for a specific FAA APT CSV archive; otherwise the current cycle is discovered automatically. |

From the setup panel, enter Space-Track credentials and choose whether to remember them. Credentials are held in server memory for the running process. Remembering them stores encrypted data in a 30-day `HttpOnly`, `SameSite=Strict` cookie; its encryption key and catalog cache are retained in `data/cache/space-track/`. Compose bind-mounts that directory, so a catalog downloaded by `space-track-test.sh` is available when the Docker server starts. Plaintext credentials are never returned by the server.

The service loads a valid cached GP snapshot on startup, labels objects for map rendering, and pins its checksum to each game. An explicit Space-Track sign-in attempts to download and atomically save a replacement snapshot. If that refresh fails, the existing cache remains playable and the UI marks it as cached while showing the refresh error. A snapshot becomes marked stale after one hour, but staleness does not prevent a game from using it. The synchronization cooldown is one hour **after a successful persisted download only**. Failed authentication, authorization, network, rate-limit, or catalog parsing attempts can be corrected and retried without triggering that local cooldown.

## Offline space-card enrichment

Enrichment never runs during server startup or Space-Track synchronization. After a snapshot is pinned locally, run:

```sh
cargo run -p sim-catalog --bin space-card-enrich
```

The command reads `data/cache/space-track/latest.json`, applies the committed rules and reviewed overrides in `data/space-cards/`, and writes the ignored runtime tree `data/generated/space-cards/`. Pass `--refresh-sources` to refresh the configured public CelesTrak and GCAT downloads under the ignored `data/cache/space-sources/` tree before generation; otherwise the last cached source versions are used. Use `--validate-only` to check full-catalog coverage without writing. Production Compose mounts the generated tree read-only; if it is missing or its checksum does not match the pinned snapshot, the API serves an explicitly uncommandable baseline card from Space-Track fields.

## Airport and runway catalog

The server loads `data/cache/airports/latest.json` immediately and refreshes stale data in the background. The worldwide baseline comes from the nightly public-domain OurAirports airport and runway CSVs. The current FAA 28-day NASR APT archive overlays U.S. runway geometry, declared distances, military/joint-use metadata, pavement classification, and reported gross-weight limits. DAFIF is not fetched because its NGA distribution requires authenticated access.

Refresh explicitly with:

```sh
cargo run -p sim-catalog --bin airport-cache-sync
```

The REST API exposes catalog status at `/v1/airport-catalog/status`, paginated search at `/v1/airports`, airport/runway details at `/v1/airports/{airport_id}`, and conservative runway compatibility evaluation at `/v1/airports/{airport_id}/compatibility`. Airport search accepts `west`, `south`, `east`, and `north` degree bounds for viewport loading, including bounds that cross the antimeridian. Optional `horizon_latitude`, `horizon_longitude`, and `horizon_radius_deg` parameters further restrict results to a spherical camera-horizon cap. The Cesium operational map uses both filters to display a compact, globe-occluded crossed-runway airport symbol and prioritizes major airports when a viewport contains more than 500 facilities. Compatibility requests supply aircraft mass, landing-gear category, operation, and already-adjusted required distance. Missing pavement-strength information returns `unknown` rather than assuming compatibility.

## Gameplay and authority workflow

1. In the scenario lobby, connect Space-Track to refresh and save the catalog; if a previous catalog is cached, it remains available when refresh fails.
2. Create **Global Crisis**, claim an available Blue role, and start the game as host.
3. Use **Configure authorities** to inspect or edit the host-managed authority graph and policies. The saved definition uses optimistic versioning to prevent accidental overwrite.
4. Submit an order. A policy can execute it directly or create an authority request for the configured approvers. Vacant approver roles resolve deterministically after their configured delay.
5. Participants see their command-chain view and relevant authority-request inbox; the Cesium map receives periodic state updates and a game-pinned orbital catalog.

## Repository layout

- `crates/sim-core/` — deterministic ECS simulation, projections, orders, and authority model.
- `crates/sim-scenario/` — validated, versioned scenario definitions and the Global Crisis fixture.
- `crates/sim-ai/` — constrained Red patrol planner that operates on a role projection.
- `crates/sim-catalog/` — provenance-aware platform, space, airport/runway, importer, and compatibility data types.
- `crates/server/` — Axum API, game lifecycle, credential cookie, catalog service, and simulation loop.
- `web/` — React, TypeScript, Cesium, persistent MIL-STD-2525D entity rendering, authority and space-asset workspaces, and Vitest frontend regression tests.
- `deploy/nginx/` — production and development edge-proxy configurations.
- `docker-compose.yml` — production-shaped local stack; `docker-compose.dev.yml` — hot-reload override.

## Verify changes

```sh
cargo fmt --check
cargo check
cargo test

cd web
npm test
npm run build
```

For Compose-only validation:

```sh
docker compose -f docker-compose.yml -f docker-compose.dev.yml config
```
