# World At War

World At War is a server-authoritative, low-fidelity war-simulation prototype. It combines a Rust entity-component simulation, a Cesium/React operational map, a public Space-Track orbital catalog, and an authority workflow for command decisions.

The scenarios include **Regional Joint Campaign** (offline collaborative plans, airspace orders and mission execution), **Contested Combat Exercise**, **Combat Training Exercise**, **Command Link Exercise**, **Sensor Relay Exercise**, **Global Crisis** (64 authored entities and a pinned public orbital snapshot), and **Jammed Flight Test**. Authored entities and uncertain tracks use MIL-STD-2525D symbols.

The broader target architecture, planned simulation fidelity, and acceptance criteria are in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md). Features described there are not necessarily implemented yet.

## What is implemented

- Delivery-gated execution acknowledgements for player orders, with four paced reply attempts, separate execution/receipt ticks, and an explicit unconfirmed outcome when replies do not arrive.
- Configured training weapons with finite ammunition, frozen reported aim points, deterministic flight delays, accumulated damage, and terminal mission success or failure.
- A deterministic Rust ECS simulation with one-second ticks, platform movement, server-side projections, and Red patrol AI that plans each aircraft once, waits for delivery, and limits retries over failed links.
- Local geometric sensor detections with spherical Earth occlusion and slant range, game/terminal-scoped track identities, and delivery-gated sensor reports. Lost contacts retain their last observed position and timestamps; the map and inspector display their age and report delay.
- Mandatory per-entity c3mesh network endpoints, bounded packet queues, deterministic loss and weighted scheduling, cyclic flight paths, geographic receiver-jamming regions, and directional link status.
- A polling multiplayer lobby, role claiming, game start/pause controls, and REST/WebSocket state delivery. Reload restores the selected game and held role without renewing its lease; temporary connection failures retry, and missing games or lost ownership return to the lobby.

- A lobby that creates and joins games, role claiming, host start/pause/resume controls, and REST/WebSocket state delivery. Paused games keep their operational map and inspectors available.
- A Cesium operational map that keeps authored owned units and uncertain tracks visually separate from the public orbital catalog, reconciling entities in place so movement ticks do not recreate or flicker MIL-STD-2525D icons.
- A lazy full-screen space-asset workspace with worker-based bulk propagation, point-primitive rendering, UTC playback, search/facets, sourced payload cards, and authority-routed satellite requests.
- A versioned authority definition: roles, operational/support/advisory/transmit relationships, policies, direct grants, approval sequences, vacant-role resolution, and human approval or denial of requests.
- A Space-Track GP catalog integration with encrypted remembered credentials, cached snapshots, clear diagnostics for credential/access/service failures, and per-game catalog pinning.
- A global airport/runway cache using public-domain OurAirports data with an authoritative FAA NASR overlay for U.S. facilities, declared distances, pavement ratings, and reported gross-weight limits.
- A Docker Compose edge proxy that serves the web client and routes `/health`, `/v1/`, and WebSocket traffic to the Rust server.
- A versioned public-safe communications catalog, per-game seed/policy/checksum pinning, append-only message events, and role-filtered map and full-screen network views.
- Campaign planning uses tick-driven application transport with 512-byte fragments, reassembly, three attempts spaced five ticks apart, independent return-path acknowledgements, expiry, and duplicate suppression. Orders execute after delivery; recipients cannot inspect undelivered command messages.
- A joint planning workspace for editing campaign intent, objectives, phases, component support, missions and airspaces; comparing courses; proposing amendments; and publishing versioned tasking over communications.
- An ACO text/file importer with UTC-to-tick anchoring, per-record preview and resolutions, explicit exclusion, revision-checked draft application, and source provenance. Operational airspaces follow each delivered activation period.
- Delivered clearances, controller handoffs, lost-communications procedures, mission reports, and map overlays. Training combat models abstract fuel/ammunition, seeded weapon outcomes and binary damage; effect assessments use the receiving role's observed tracks.
- Bounded role-authorized runtime diagnostics for tick/projection costs, state-request freshness, map processing and visible queues; an offline release benchmark with repeat and baseline projection-hash checks.
- Earth-horizon sensing, scan intervals and field of regard, aging tracks, and campaign-specific delayed friendly-position/contact reports. Training exercises retain their authored friendly-unit overview and explicit sensor-report subscriptions.

Current limitations: controller ground-truth privileges, durable packet-level history, crash recovery, and full replay remain planned. The message log records server C2 lifecycle events; internal knowledge reports and individual fragments are not a complete durable packet audit. Sensor, airspace and combat behavior uses simplified training estimates: airspaces use shared spherical geometry and explicit MSL bounds, while the Cesium altitude rendering remains approximate; unapproved entry is reported as a violation. The broader terrain, logistics, cyber, multi-domain platform and multi-source catalog systems remain planned work. The pre-merge campaign implementation measured Global Crisis core tick p95 at 52–58 ms on the documented development hardware; those historical numbers and hashes do not certify this integrated implementation, and full server/client load and roadmap-scale budgets remain uncertified. See [performance measurements and reproduction commands](docs/performance/README.md).

[Verified gameplay screenshots](docs/gameplay-checkpoints.md) show firing authorization, mission completion, delivered sensor knowledge, executed movement, network inspection, and map loading.

## Prerequisites

- Rust 1.89 or newer for the Rust 2021 workspace and its locked dependencies.
- GitHub read access to [c3mesh](https://github.com/dancarpenter21/c3mesh), checked out as the sibling `../c3mesh` at the v0.2.0 release commit in [c3mesh-revision.txt](c3mesh-revision.txt). The setup scripts clone the dependency when missing and select the pinned revision when its working tree is clean.
- Node.js 22+ and npm for frontend development.
- Docker Compose v2 for the container workflows.
- A Space-Track account only when creating a scenario that requires the public orbital catalog.

## Run locally

Prepare the pinned Rust dependency once from the repository root:

```powershell
./scripts/setup-c3mesh.ps1
./scripts/setup-c3mesh.ps1 -VerifyOnly
```

On Linux or macOS, use `sh scripts/setup-c3mesh.sh` and
`sh scripts/setup-c3mesh.sh --verify-only`. Setup preserves branches and refuses
to switch a dependency with local changes. When working on both projects, keep
the sibling checkout on your development branch; update the pinned revision
after its tested changes are pushed. The verification mode requires the exact
clean checkpoint used for reproducible builds.

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

Open `http://localhost:5173`. For a first game, select **Command Link Exercise**, create the game, claim **Exercise Commander**, and start the scenario. Submit movement orders for both aircraft, then open **Network** to inspect their queue and delivery states. The shared 2.4 kbit/s net permits four waiting packets per directional link in addition to a packet already transmitting; excess packets fail with a recorded queue drop. Pause/resume retains pending radio traffic. The exercise uses fictional positions and deliberately slow radio settings, runs without an orbital catalog, and keeps the base stationary.

The Vite development server proxies API calls to `http://localhost:8000` by default. Set `VITE_API_BASE` in the frontend environment (for example, `VITE_API_BASE=https://api.example.test npm run dev`) only when the browser must use a different API origin.

## Run with Docker

The default Compose file is production-shaped: it builds a release Rust server and static web assets, then exposes only the Nginx edge proxy.

```sh
docker compose up --build
```

Open `http://localhost:8080`, or the port set by `APP_PORT`. The internal server and web containers are not published directly.

For containerized development with hot reloading, use the development override:

```sh
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build --watch
```

This workflow requires Docker Compose 2.32 or newer. The server image receives the sibling `c3mesh` checkout through a named build context, and Compose Watch syncs changes from both Rust projects. It also syncs browser assets into the Vite container, restarts Nginx when its development configuration changes, and rebuilds the affected image when a dependency manifest or Dockerfile changes. Named volumes preserve Cargo artifacts/downloads and the Space-Track cache between restarts. Stop either stack with the matching `docker compose ... down` command.

Application, dependency, proxy, and image-definition changes are handled for the running stack. Changes to either Compose YAML file alter the watcher itself, so restart the command after editing those files.

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
| `COMMUNICATIONS_CATALOG_PATH` | Versioned public-safe equipment, message, and network-policy catalog; defaults to `data/communications/catalog.yaml`. |
| `NETWORK_EVENT_DIR` | Writable append-only per-game network event directory; defaults to `var/network-events`. A write failure pauses the affected game. |

From the setup panel, enter Space-Track credentials and choose whether to remember them. Credentials are held in server memory for the running process. Remembering them stores encrypted data in a 30-day `HttpOnly`, `SameSite=Strict` cookie; its encryption key and catalog cache are retained in `data/cache/space-track/`. Compose bind-mounts that directory, so a catalog downloaded by `space-track-test.sh` is available when the Docker server starts. The remembered username may be returned to populate the setup form; the plaintext password is never returned by the server.

The service loads a valid cached GP snapshot on startup, labels objects for map rendering, and pins its checksum to each game. An explicit Space-Track sign-in attempts to download and atomically save a replacement snapshot. If that refresh fails, the existing cache remains playable and the UI marks it as cached while showing the refresh error. A snapshot becomes marked stale after one week, but staleness does not prevent a game from using it. The synchronization cooldown is one hour **after a successful persisted download only**. Failed authentication, authorization, network, rate-limit, or catalog parsing attempts can be corrected and retried without triggering that local cooldown.

The browser suite uses isolated Rust servers, separate player contexts, an offline airport fixture, and a local Space-Track-compatible provider. It covers campaign delivery and role-scoped visibility, reload and WebSocket recovery, ACO imports, runtime diagnostics, clearances, controller handoffs and mission cancellation. The setup-form test also checks the encrypted credential-cookie flow and persisted two-object GP snapshot. Run the suite in the dedicated test container, which pins Chromium to the project's Playwright version:

```sh
docker compose --profile test run --rm --build e2e
```

To perform the same test against the live provider, explicitly set `SPACETRACK_E2E_USERNAME` and `SPACETRACK_E2E_PASSWORD`. Live mode makes a real full-catalog GP request, so run it no more than once per hour and never commit or print those values.

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

## Sensor model

The generic prototype sensor uses straight-line distance between the observer and target, including altitude. Detection also requires the line joining those positions to clear a sphere with mean radius 6,371 km. Each endpoint contributes its own geometric horizon angle, so a climbing aircraft can become visible before its surface position changes. The calculation follows the tangent geometry described in NASA's [Distance to the Horizon](https://cdaweb.gsfc.nasa.gov/pub/documents/archived_websites/pwg.gsfc.nasa.gov/stargaze/Shorizon.htm), with a submillimeter angular tolerance at tangency.

Platform altitude is the sensor/target height above that reference sphere; no separate antenna offset is assumed. Terrain, atmospheric refraction, weather, signatures, scan patterns, probabilistic measurement error, and underwater sensor modalities remain planned. Sensor ranges and initial coordinates must be finite and valid. Identification confidence remains a simple range threshold and does not disclose the target's actual platform symbol.

Observations update only the detecting unit's local knowledge. A different command terminal does not automatically inherit those tracks. When detection stops, the track keeps its last position and observation/receipt ticks rather than following the hidden target. Track IDs are scoped to the game and terminal instead of exposing enemy entity IDs. The inspector separates observation age, time since receipt, and delivery delay; map tracks fade when they represent an earlier observation. Confidence decay, uncertainty growth, track expiry, and multisensor fusion remain planned.

The authored [Sensor Relay Exercise](data/scenarios/sensor-relay-exercise.v1.json) opts CAP Alpha 1 into reporting to the exercise command post once every ten ticks per currently observed track. The pilot sees the contact locally; the commander receives a snapshot after its packet arrives; CAP Alpha 2 receives no automatic knowledge. The fictional target periodically leaves sensor range, making the difference between current observations and retained tracks visible. No orbital catalog or provider credentials are needed.

Report routes are explicit same-side subscriptions with validated sensing roles, recipients, links, and positive intervals. Reports use the frozen public-safe track-report message profile and compete with orders on the shared training radio. At most one report from a source track to a recipient is pending; failed attempts are paced, and the next attempt samples a fresh local observation. An expired report terminates at its deadline even on a very slow link, while already reserved physical serialization remains non-preemptive.

The recipient's state, network view, message detail endpoint, REST event history, and network stream withhold incoming report contents until delivery is persisted. Dropped, expired, or failed-persistence reports do not update the recipient's knowledge. Received tracks retain their original observation tick, use the actual receive tick, and never read the target's current truth. Older or duplicate reports cannot overwrite a newer report from the same source. Report routes are disabled in the other existing scenarios; automatic forwarding and multisensor fusion remain planned.

## Combat training

Select **Combat Training Exercise**, create a game, claim **Exercise Commander**, and start. CAP Alpha 1 detects the nearby hostile drone; its report must arrive at the command post before the commander can select the contact under **Engagement orders**. Send the order and watch its delivery and launch receipt. A successful impact completes the objective and pauses the scenario permanently. Leave and create a new game to retry.

A second player can claim **Pilot, CAP Alpha 1** and request a shot using local sensor knowledge. The commander opens **Authorities → Requests** to approve or deny it. The request appears only after its radio packet arrives and shows the frozen aim point, confidence, and observation tick. Incoming message details also withhold those coordinates until recorded delivery. Approval still requires the final firing order to arrive at the aircraft. Denied, dropped, expired, out-of-range, or stale orders consume no ammunition. Retrying a lost submission response reuses the original order ID and cannot fire a second shot.

The [authored exercise](data/scenarios/combat-training.v1.json) provides two fictional shots, a 12 km launch range, a 15-tick maximum report age, and a 90-tick deadline. Reports require at least 80% identity confidence. The original reported position and observation tick are frozen into the command; report age and range are checked again at launch. A weapon has no seeker or guidance: it resolves a deterministic delayed impact at that aim point, so a moving contact can escape. Its configured radius damages only configured enemy platforms. Damage accumulates, and zero hit points remove the platform from movement and sensing.

**Weapon launched** confirms execution, not a hit. Impact telemetry is local to the firing terminal; other terminals do not automatically receive it. The exercise's public adjudicator announces success or failure to the objective's side, without exposing enemy entity IDs, names, or remaining health. Destroying the target succeeds; exhausted shots or the time limit fail. Terminal outcomes stop the server clock, reject resume, and remain available after reload or rejoin.

Weapon and durability definitions are optional scenario data; the existing exercises remain unarmed. These are deliberately simple training mechanics. Guided interception, seekers, real weapon envelopes, countermeasures, friendly fire, fuel, and a general campaign objective system remain planned. Active games and combat state still live in memory and are lost on server restart.

## Contested combat and mission debrief

Create **Contested Combat Exercise**, claim **Exercise Commander**, and start. The command post is jammed for ticks 0–7, so initial sensor reports can fail. When the blackout ends, fresh reports arrive on the shared radio. The drone patrols between two positions and holds from ticks 20–54; use the observation age and changing reported positions to reacquire it before firing. An early shot at a moving contact can miss its frozen aim point. CAP Alpha 1 has three fictional shots and the objective has a 150-tick deadline. Movement orders remain available to reposition the aircraft.

The pilot's impact telemetry is explicitly subscribed to the command post. **Received impact report** appears only after its packet delivery is recorded; dropped, expired, or unpersisted packets reveal no result. Reports contain the original observation, launch, and impact ticks plus hit/miss, without enemy identity or remaining health. Each route makes at most four attempts, paced five ticks apart in this scenario. The other pilot receives no report automatically.

Open **Mission debrief** during or after either combat exercise. Its table separates observation, approval, final order delivery, launch, impact, and report receipt. The radio history shows visible packet outcomes. Unknown events remain blank: a commander who approved a pilot's order does not gain local impact telemetry automatically, and a received report does not expose another player's unreported submission history. Access requires the current player and role lease; reloading or rejoining preserves the in-memory timeline.

After the contested objective ends, combat time, movement, sensing, and new orders stop. Only the radio clock advances for at most 20 ticks so final impact reports and execution acknowledgements can arrive or exhaust their attempts; the game then pauses. Packets still pending at the limit receive recorded drops. The interface marks this phase **Radio reporting**, and the debrief displays combat and radio clocks separately. The original Combat Training Exercise retains its immediate pause and local-only impact telemetry.

Jamming regions can optionally specify an inclusive `active_from_tick` and exclusive `active_until_tick`. Omitted values preserve continuously active regions. Exercise timings, patrols, report subscriptions, and limits live in [contested-combat.v1.json](data/scenarios/contested-combat.v1.json). These rules use the existing pinned c3mesh packet engine and add no external data source. Save/load and durable replay remain future work.

## Gameplay and authority workflow

1. Select a scenario. **Global Crisis** requires a usable Space-Track catalog; **Jammed Flight Test** and **Regional Joint Campaign** do not.

The exercise definition in [data/scenarios/command-link-exercise.v1.json](data/scenarios/command-link-exercise.v1.json) keeps units, authority relationships, provenance, and bounded radio settings separate from simulation code. Its checked parser builds the packet topology and validates the authority chain. The data is embedded at compile time; Docker builds include it and the development watcher recompiles when it changes.

Use **Movement orders** to choose a commanded unit, a clockwise course from north (0–360°), and speed (0–1,000 m/s). **Turn north** sends a 130 m/s northbound order; **Stop unit** cancels movement after its command arrives. The panel prefers an aircraft over a command base and reports packet delivery, authority approval, and execution confirmation. The execution tick and confirmation receipt tick appear only after the executor's acknowledgement reaches the issuing terminal. On a narrow screen, open **Commands** to access the same controls while retaining the map. A lost or timed-out response exposes **Retry order**, which sends the original ID and payload and can recover an accepted order while the scenario is paused.

Movement uses continuous great-circle motion on the same mean-radius Earth as the sensor model. A commanded course defines the initial north/east tangent direction at execution; the route continues through a pole or across the dateline without invalid coordinates or a direction reversal. Changing course starts a new arc at the current position, and stopping holds that position. Horizontal speed uses the starting altitude; climb is applied independently and altitude is bounded at the reference surface. This is a kinematic prototype without acceleration, fuel, or flight dynamics.

Authored cyclic paths interpolate the short spherical arc between waypoints, with linear altitude interpolation. Dateline-crossing routes therefore stay near the dateline. Exactly antipodal waypoints use a deterministic northward arc because their shortest route is ambiguous; intermediate waypoints should specify the intended route.

Owned-unit projections include the last commanded velocity and whether an authored flight path still controls motion. The Red planner uses reports at its issuing role terminal, assigns overlapping command scopes to the most specific AI role, and leaves non-aircraft units stationary. It submits a new patrol only when the desired movement changes, waits for pending delivery or approval, and retries unchanged failures no more often than every ten simulation ticks.

1. Select a scenario. **Global Crisis** requires a usable Space-Track catalog; **Jammed Flight Test** does not.
2. Create the game, claim an available command or pilot role, and start it as host.
3. Use **Configure authorities** to inspect or edit the host-managed authority graph and policies. The saved definition uses optimistic versioning to prevent accidental overwrite.
4. Submit an order. A policy can execute it directly or create an authority request for the configured approvers. Vacant approver roles resolve deterministically after their configured delay.
5. Participants see their command-chain view and relevant authority-request inbox; the Cesium map receives periodic state updates and a game-pinned orbital catalog.
6. The host can **Pause scenario** or **Resume scenario** from the header, including on narrow screens. All players retain their map while paused, and movement orders are disabled until the game is running.

Orders sent to another entity stay pending until their packets arrive; serialization, queue waits, jamming, MTU rejection, loss, and expiry affect whether execution becomes possible. The catalog's message priority, forwarding-hop budget, and expiry are carried into the packet engine. A local order at the same command entity can be delivered immediately. An accepted submission reports that the order was queued, and a delivered order enters the executor on the next simulation tick.

Leaving returns to the lobby and keeps your server role reserved. To return, choose **Join game**, select the game, then select the role marked **your role**. Reclaiming obtains a fresh lease that invalidates older command submissions; it does not start a paused scenario. Other players' held roles remain disabled. The role-list endpoint accepts an optional `player_id` query and reports `held_by_you` without returning the owner's identity; projection and command endpoints still validate the caller's role access.

Player orders submitted to `POST /v1/games/{game_id}/roles/{role_id}/intent` use `intent_id` as an idempotency key for the running game. Retrying the same command returns its original submission result without sending another packet, including when the game has since paused. Reusing an ID for a different command returns `409 intent_conflict`. Both retries and receipt reads require the current role lease; a new holder cannot read the previous player's receipts. New movement orders require finite vectors with speed at most 1,000 m/s.

Read `GET /v1/games/{game_id}/roles/{role_id}/intents/{intent_id}?player_id=...&lease_generation=...` for queued, in-transit, approval, execution, or terminal failure status. A remote delivered order stays `awaiting_acknowledgement` before and after execution until its reply arrives; the receipt withholds the executor's success, rejection reason, and `executed_tick` throughout that wait. A recorded reply adds `acknowledged_tick`, the radio tick when the issuing terminal received it.

Execution acknowledgements use the existing public-safe acknowledgement profile and originate at the executing unit. They compete with other radio traffic and make at most four attempts, at least ten radio ticks apart, with only one reply pending for an order. Retrying a reply never resends or reexecutes the command. Reply loss and expiry leave the original execution intact. Incoming replies are absent from the issuer's network projection, message detail, event history, and stream until delivery is persisted. A local execution records its acknowledgement without a physical packet; a failed write still grants no confirmation.

An unanswered receipt becomes `unconfirmed` when the completed exercise closes its radio window or when the issuer's observable confirmation timeout elapses. The timeout is four acknowledgement expiry intervals plus three retry intervals (510 radio ticks with the committed profile), measured from command delivery. This state reports missing confirmation and does not claim execution or rejection. A subsequently delivered reply can still confirm the original result. The contested exercise's finite reporting window also settles execution acknowledgements and records terminal drops at closure; Combat Training retains its immediate mission-end pause. The debrief separates launch time, acknowledgement receipt, impact time, and impact-report receipt. Command packets retain their original intent ID, movement vector, and requested tick in structured fields. Receipts and the deduplication ledger live with the in-memory game; durable network history remains in the append-only event file.

Authority requests and approvals use the same transport. A remote approver receives the request before a human decision becomes available or a vacant-role timer starts. Final approved orders also wait for delivery. Pausing freezes both packet progress and execution. A lifecycle-event write failure pauses the game and cancels pending delivery actions so an unrecorded command cannot execute.

The lobby loads its own small client bundle. Cesium, map symbols, and orbital rendering load when a playable map opens; authority and network workspaces also load on demand. A slow map download shows a loading state while host controls remain available. A failed download offers a page reload. The browser remembers the selected game, player, role, and lease generation, checks that lease against the current server roles, and lets the state endpoint authorize access before displaying an operational picture. Reloading a paused game does not resume it; leaving, changed leases, and rejected access clear the saved selection. Invalid saved data cannot block the lobby.

Production builds emit a Vite manifest and enforce a 350 KiB limit on all startup JavaScript, including static imported chunks. Dynamic map and graph bundles are excluded from that startup limit. The split reduced the initial minified script from 5.26 MB to about 239 KB in the current build; Cesium's separate map bundle remains substantial.

Each session resource waits for its previous refresh to finish, cancels obsolete reads when a game or role changes, and times out after ten seconds. Temporary failures retain the last map with a visible connection notice, disable movement orders, and retry with bounded backoff. **Retry connection** requests a fresh update immediately. A rejected or changed role lease removes the operational picture and returns the player to role selection. Leaving a game cancels pending reads and host controls so delayed responses cannot reopen it.

### Regional campaign planning

Create **Regional Joint Campaign**, claim **Joint Force Commander**, start the scenario, and open **Joint planning**. Compare the two authored courses, edit the draft, save changes, then approve and publish the selected course. Publication queues a separate message for each allied role; receiving a plan starts only that unit's assigned tasking. Component commanders can save proposals and send them back for commander adoption. Save an amendment before publishing a new revision.

In another browser session, claim a pilot or sector-controller role to receive the plan, request/grant clearances, offer/accept handoffs, and inspect delayed mission reports. An accepted handoff takes effect when its message reaches the aircraft. Map airspaces and routes come from the received plan, never the unpublished draft. Remote friendly positions and tracks may be stale during interference.

Open **Import airspace order** in the planning workspace to paste an ACO or select a text file. Enter an explicit UTC anchor, corresponding simulation tick, year, and UTC horizon. Preview the order, map controllers and airspace kinds, resolve unsupported vertical bounds to metres MSL and activation windows to tick ranges, or explicitly exclude invalid records. Geometry errors require correcting the source or excluding the record. Preview again after changes, then apply to the saved draft. Application does not publish the order. Imported boundaries and activation periods are amended through the source import; normal campaign saves preserve their geometry and provenance.

ACO endpoints are `POST /v1/games/{id}/roles/{role_id}/planning/aco/preview` and `/apply`. Both accept `player_id`, `lease_generation`, `expected_revision`, `source`, and `options` (`anchor_utc`, `anchor_tick`, `year`, `horizon_end_utc`, and per-external-ID `resolutions`). Apply rejects invalid or stale requests without modifying the draft. Reimporting an unchanged message preserves IDs and the draft revision. The supported dialect is fixture-defined in `data/aco/labelled.aco`; this is not a universal ACO parser.

The planning API is `GET /v1/games/{id}/planning?player_id=...&role_id=...` and `POST /v1/games/{id}/roles/{role_id}/planning`. Mutations require `player_id`, `lease_generation`, and a tagged `action`: `save`, `propose`, `adopt_proposal`, `publish`, `request_clearance`, `grant_clearance`, `offer_handoff`, `accept_handoff`, or `cancel`. Saves and publication use `expected_revision`; published revisions are immutable. Fixture plans, defensive tasking and explicitly estimated combat parameters live in `data/scenarios/`.

## Communications catalog and network APIs

Validate the committed catalog and print its checksums and entity coverage with:

```sh
cargo run -p sim-comms --bin comms-catalog-validate -- data/communications/catalog.yaml
```

The structural schema is checked in at `data/communications/schema/catalog.schema.json`; startup also performs semantic validation for duplicate IDs, unresolved references, estimates without rationale, invalid bands/rates, and empty queues. Game creation accepts optional `seed` and `network_policy_id` fields and returns the pinned scenario version, catalog and message-pack checksums, seed, and policy in the game summary.

Role-held network access is available at `/v1/games/{id}/network`, the sequenced WebSocket `/v1/games/{id}/network/stream`, cursor-paginated `/v1/games/{id}/network/events`, and authorized `/v1/games/{id}/network/messages/{message_id}`. Message content is limited to originating roles and destination roles. The event endpoint paginates immutable queued, in-transit, and terminal transitions; projections contain one current record per message. Stream sequence numbers track network revisions independently of the simulation tick.

The operational map previews up to eight links involving your terminal or commanded units, placing failures and queued traffic first. Its summary covers all monitored links; **Inspect full network** opens the network workspace. Cesium redraws when units, map layers, or the camera change, caps interactive rendering at 30 frames per second, suspends rendering beneath a full-screen workspace, and preserves the map and camera when you return.

The **Network** workspace starts with directional connections involving your role's terminal, with the current focus named in its summary. **All connections** opens the complete role-visible graph; **My terminal** returns to the issuing terminal. This keeps the 48-terminal full-mesh scenario's initial view at 94 links instead of 2,256, reducing its measured rendered DOM from about 9,800 to 1,200 nodes. Focusing the topology does not restrict the role's authorized message history.

The **Network** workspace searches role-visible terminals by name or domain and filters directional links by availability, jamming, or queued traffic. Select a terminal to inspect its connections, focus its neighborhood, or browse messages to and from it. Selecting a link scopes message history to that exact direction. The message inspector shows authorized content, structured fields, delivery timing, queue wait, network transit time, classification, and drop reasons; history can be searched and filtered by lifecycle state.

Dragged terminal positions remain in place across live updates and filtering. **Fit view** frames the current filters; **Reset layout** restores the grid. A disconnected stream retains the last topology with a reconnecting notice, and malformed updates trigger a fresh snapshot. On narrow screens, the graph and inspector stack vertically. Press **Escape** or choose **Back to map** to close the workspace.

The standalone browser regressions use deterministic role-visible network fixtures and do not require a running Rust server, Docker, or Space-Track credentials:

```sh
cd web
npx playwright install chromium
npm run test:e2e:network
```

The standalone session tests exercise the actual Cesium map with mocked REST responses, covering host and guest pause/resume, retained maps during outages, request timeouts, revoked roles, delayed responses after leaving, mobile controls, keyboard camera movement while paused, and a bounded communications preview across 4,032 directional links:

```sh
cd web
npm run test:e2e:session
```

The real-server gameplay regression creates Global Crisis through the browser, sends a networked command, checks lifecycle events in both the API and JSONL store, verifies role access, inspects message timing, and exercises host pause/resume. It also attaches per-stage elapsed time and DOM counts for diagnosing client performance. Its Space-Track and airport providers use small local fixtures in an isolated runtime directory:

```sh
cd web
npm run test:e2e:gameplay
```

**Continuous integration** checks the pinned c3mesh checkout, formatting, strict lints, and workspace tests on Linux and Windows using Rust 1.89.0 and current stable. The Linux web job uses the npm lockfile, builds the production client, and runs all browser tests with local catalog providers. Browser reports, screenshots, and failure traces are retained for seven days. See [.github/workflows/checks.yml](.github/workflows/checks.yml).

The [production container workflow](.github/workflows/containers.yml) runs when container definitions, dependency pins, authored scenario data, or smoke inputs change, and can also be dispatched manually. It builds the server and client with their lockfiles, starts the production Compose edge stack, and verifies static assets, a catalog-free game, an identical order retry, recorded packet delivery, actual movement execution, and pause through the proxy. The stack is always stopped afterward. Docker contexts exclude local builds, test reports, tool state, and environment files.

After starting a local stack, run `node scripts/smoke-command-exercise.mjs --base-url http://127.0.0.1:8080` for the same smoke check. It creates a game named **Production smoke exercise** and leaves it paused. Pass `--skip-web` when checking a direct server without an edge proxy.

## Repository layout

- `crates/sim-core/` — deterministic ECS simulation, projections, orders, and authority model.
- `crates/sim-scenario/` — validated, versioned scenarios and the authored command exercise loader.
- `crates/sim-ai/` — constrained Red patrol planner that operates on a role projection.
- `crates/sim-catalog/` — provenance-aware platform, space, airport/runway, importer, and compatibility data types.
- `crates/sim-comms/` — communications catalogs, validation, checksums, and public-safe C2 message types.
- `crates/server/` — Axum API, game lifecycle, credential cookie, catalog service, and simulation loop.
- `web/` — React, TypeScript, Cesium, persistent MIL-STD-2525D entity rendering, authority and space-asset workspaces, and Vitest frontend regression tests.
- `deploy/nginx/` — production and development edge-proxy configurations.
- `docker-compose.yml` — production-shaped local stack; `docker-compose.dev.yml` — Compose Watch hot-reload override.

## Verify changes

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd web
npm test
npm run build
npm run test:e2e
```

Browser tests require Playwright Chromium. The seven campaign integration tests use real application APIs and simulation delivery; they only replace external map imagery and deliberately interrupt requests/streams for recovery checks. The smaller planning contract test retains API fixtures. Host runs write planning/import screenshots and failure traces under `web/test-results/`; view a failure trace with `npx playwright show-trace <trace.zip>`. The Space-Track test uses a local mock unless both live-provider test variables are explicitly set. Tests never read or modify the development stack's catalog cache.

To retain artifacts from a container run, omit `--rm`, name the test container, and copy its results afterward:

```sh
docker compose --profile test run --name world-at-war-browser-tests --build e2e
docker cp world-at-war-browser-tests:/workspace/web/test-results/. web/test-results/
docker rm world-at-war-browser-tests
```

For an interactive Windows Chrome debug session from WSL, use PowerShell to start the installed Windows executable with a dedicated profile (after starting the development stack):

```powershell
$chrome = "C:\Program Files\Google\Chrome\Application\chrome.exe"
$profilePath = Join-Path $env:LOCALAPPDATA "WorldAtWarChromeDebug"
Start-Process -FilePath $chrome -ArgumentList @("--remote-debugging-port=9222", "--user-data-dir=`"$profilePath`"", "--no-first-run", "http://localhost:8080")
```

Chrome exposes CDP at `http://localhost:9222` on Windows. Access it through Windows tooling when WSL cannot reach Windows loopback. `PLAYWRIGHT_WS_ENDPOINT` remains a Playwright-protocol endpoint, not a Chrome CDP URL. The dedicated profile starts without your normal browser's remembered Space-Track cookie; use the original browser profile to restore previously saved credentials, or configure the ignored root `.env` for Compose. Role recovery likewise uses browser-local identity and an in-memory server; it does not implement authenticated resume tokens or recovery after a server restart.

For Compose-only validation:

```sh
docker compose -f docker-compose.yml -f docker-compose.dev.yml config
```
