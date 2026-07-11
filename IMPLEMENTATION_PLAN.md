# World At War Implementation Plan

## 1. Product Contract

World At War is a persistent, server-authoritative, multiplayer command simulation. The first complete release models a feature-rich Blue force across air, space, cyber, sea, undersea, and land. Red is controlled by a basic deterministic AI that uses the same imperfect information and legal command interface as a human player.

The game is low fidelity in presentation and computational detail, but not in causal structure. A sensor observation is not shared knowledge. A valid track is not necessarily identifiable. A player order is not automatically authorized, delivered, understood, or executable. Those distinctions are the core game.

### Required player journeys

1. A player signs in, lists running and joinable games, inspects available roles, joins a game, claims a role, receives only that role's current picture, and can reconnect without losing the role.
2. A player creates a game from a scenario, chooses seed/time controls and optional slots, waits in a lobby, starts it, and may pause or end it if authorized.
3. A national-command player issues an order through a modeled authority and communications chain.
4. An operational or tactical player receives, acknowledges, delegates, rejects, or executes an order according to authority and capability.
5. A unit operator controls an assigned unit but sees only local observations plus reports delivered over functioning communications.
6. An observer or game controller can inspect ground truth only when the scenario grants that privilege.

### Release boundaries

- **Blue:** every system in this document is implemented and usable: multi-echelon authority, communications, uncertain tracks, all six domains, logistics, movement, weapons, electronic warfare, cyber effects, orbital motion, airports, terrain-aware line of sight, and role-scoped multiplayer.
- **Red:** scenario-authored order of battle, the same platform/component model, the same sensing and communications constraints, and a basic goal-oriented AI. Red does not need a human-facing command workflow, sophisticated doctrine learning, diplomacy, or deceptive natural-language behavior for the first release.
- **Not a first-release goal:** classified accuracy, real-world operational planning, photorealistic rendering, a public scenario marketplace, mobile-native clients, or live military feeds beyond public orbital and geographic catalogs.

## 2. Opinionated Technical Architecture

### Stack decision

- Rust 2021 workspace, with **standalone `bevy_ecs`** as the authoritative simulation kernel. Bevy ECS is explicitly usable without the Bevy renderer and provides typed components, resources, relationships, change detection, ordered schedules, and parallel system execution ([Bevy ECS documentation](https://docs.rs/bevy_ecs/latest/bevy_ecs/)). Pin an exact compatible crate version in `Cargo.lock` and upgrade deliberately.
- `tokio` + `axum` for HTTP/WebSocket services; JSON for administration and MessagePack or Protobuf for high-rate state frames after measuring the JSON vertical slice.
- PostgreSQL for users, games, role leases, scenario versions, command/event history, and snapshots. The ECS world is memory-resident while a game is active.
- React + TypeScript + Vite + CesiumJS for the browser client. The client renders role-visible projections and never receives hidden ground truth.
- Docker Compose for PostgreSQL, API/simulation server, data importer, and web client.
- One simulation process may host multiple small games initially. Define a `GameRuntime` boundary so games can later be assigned one-per-process without changing protocols.

### Repository layout

```text
Cargo.toml
crates/
  sim-core/          # components, schedules, deterministic domain systems
  sim-geo/           # WGS84/ECEF/geodesy, terrain, LOS, propagation helpers
  sim-catalog/       # typed platform and sensor catalog schemas
  sim-scenario/      # scenario schema, validation, spawn/restore
  sim-ai/            # basic Red planner using role-visible state
  server/            # auth, lobby, roles, WebSocket gateway, runtimes
  data-import/       # source adapters, normalization, provenance reports
web/
  src/features/map/
  src/features/lobby/
  src/features/roles/
  src/features/orders/
  src/features/tracks/
tests/
  scenarios/
  fixtures/
data/
  schemas/
  catalogs/          # reviewed, normalized, redistributable data
  scenarios/
  provenance/
docker-compose.yml
.env.example
```

### Runtime topology

```text
Browser -- HTTPS/WSS --> Axum gateway --> GameRuntime --> bevy_ecs World
                              |               |              |
                              v               v              v
                         PostgreSQL       snapshots       event log

Importer --> source cache --> normalize/validate/review --> versioned catalogs
```

The server accepts player **intent**, validates identity/role/authority, and queues a command for a future simulation tick. It publishes per-role projections, not entity replication. All wall-clock input is converted to a monotonically increasing `Tick`; simulation systems must not read system time or unseeded randomness.

## 3. Simulation Model

### Stable identity

Bevy `Entity` values are runtime-local. Every persistent or network-visible object also has a scenario-scoped `SimEntityId` UUID. Catalog types use stable semantic IDs such as `us.af.f-35a.block4`; scenario instances refer to a catalog version and overrides. Never persist raw ECS entity IDs.

### Principal entities and components

| Entity | Important components |
| --- | --- |
| Platform/unit | `SimEntityId`, `Side`, `PlatformType`, `GeoPose`, `Kinematics`, `Mobility`, `Fuel`, `Damage`, `SignatureSet`, `Inventory`, `ParentUnit`, `Doctrine`, `AuthorityNode` |
| Person/role station | `RoleKind`, `AssignedPlayer`, `RolePermissions`, `CommandScope`, `KnowledgeBase`, `Location` |
| Sensor | `SensorType`, `SensorMode`, `Mount`, `FieldOfRegard`, `ScanPattern`, `PerformanceModel`, `EmissionState`, `PowerState` |
| Communications node | `Transceiver`, `Antenna`, `NetworkMembership`, `CryptoState`, `Queue`, `RelayPolicy`, `JammingState` |
| Weapon/effect | `WeaponType`, `Launcher`, `Guidance`, `Warhead`, `TargetReference`, `TimeToLive` |
| Ground formation | `UnitType`, `ControlArea`, `Strength`, `Cohesion`, `SupplyState`, `Posture`, `Frontage` |
| Facility | `FacilityType`, `Footprint`, `Runways`, `Berths`, `Capacity`, `Damage`, `Services` |
| Orbital object | `CatalogNumber`, `OmmElements`, `Epoch`, `OrbitState`, `PayloadCapabilities` |
| Track/report | `TrackId`, `OwnerRole`, `EstimatedState`, `Covariance`, `Classification`, `IdentityHypotheses`, `SourceHistory`, `Freshness` |
| Order/message | `Issuer`, `AuthorityClaim`, `Recipients`, `Payload`, `Preconditions`, `Classification`, `Route`, `DeliveryState`, `Acknowledgements` |
| Cyber mission | `Access`, `TargetSystem`, `Effect`, `Duration`, `AttributionConfidence`, `DiscoveryRisk` |

Use ECS relationships for mount/platform, subordinate/commander, embarked/carrier, runway/airbase, and relay/network relationships. Keep large immutable catalog records outside components and reference them by stable ID.

### World resources

`SimClock`, seeded `DeterministicRng`, `ScenarioRules`, `TerrainProvider`, `WeatherGrid`, `CatalogSet`, spatial indexes, network graphs, authority graph, pending player intents, event sink, and metrics. Resource changes that affect outcomes are recorded in snapshots/events.

### Fixed schedule

At 1 Hz initially, with configurable simulated seconds per tick:

1. `Ingress`: validate and canonicalize commands already assigned to this tick.
2. `Authority`: check issuer, delegated authority, command scope, rules of engagement, and required confirmations.
3. `CommunicationsPlan`: compute eligible links from geometry, equipment, spectrum, power, crypto, damage, and jamming.
4. `CommunicationsDeliver`: apply latency, bandwidth, queues, loss, relay policy, expiry, and acknowledgements.
5. `Orders`: accept delivered orders, check local capability/preconditions, and update tasks.
6. `Movement`: flight/ship/submarine/ground/orbital propagation and fuel use.
7. `Environment`: weather and terrain-dependent state.
8. `EmissionsAndCyber`: transmissions, jamming, cyber access/effects, and signatures.
9. `Sensors`: candidates, horizon/terrain occlusion, field of view, detection probability, measurement generation.
10. `TrackFusion`: correlate measurements, update uncertainty/classification, age or drop tracks.
11. `Combat`: launch, guidance, interception, impact, damage, suppression, and expenditure.
12. `Logistics`: consumption, transfer, repair, runway/port capacity, readiness.
13. `RedAI`: observe Red's role projection and enqueue intent for a later tick.
14. `Projection`: build role-specific deltas and notifications.
15. `Persist`: append significant events; snapshot on interval or lifecycle transition.

System-set ordering is explicit and ambiguity warnings fail tests. Parallelize only systems whose ordering cannot change observable results. Use fixed-point integers or carefully bounded integer units for authoritative distances, time, inventory, and probabilities; isolate floating-point geodesy and document tolerances.

## 4. Information, Sensors, and Communications

### Three separate states

1. **Truth:** authoritative platform state in the ECS world.
2. **Observation:** a time-stamped noisy measurement produced by one sensor, with no guaranteed identity.
3. **Knowledge:** role-owned tracks and reports derived from observations received locally or over communications.

The projection system reads only knowledge owned by or delivered to the role. A track contains an estimated position/velocity and covariance or bounded region, timestamps for observation and receipt, source type at an allowed disclosure level, and probabilistic class/identity hypotheses. Track IDs are side/role scoped and cannot be used as hidden entity IDs.

### Sensor models

- Visual/EO/IR: terrain and Earth occlusion, weather attenuation, daylight/contrast, field of view, range-dependent classification.
- Radar: radar horizon, scan revisit, target radar cross-section band, aspect modifier, clutter, emission control, jamming and burn-through approximation.
- SIGINT/ESM: emitter library match, line of bearing, multi-sensor triangulation, uncertain ELNOT region, and identity confidence; no magic exact position.
- Acoustic/sonar: active/passive modes, simplified ocean region conditions, bearing/range uncertainty, speed/noise tradeoffs, countermeasures.
- Space imagery/SIGINT: propagated orbit, sensor swath, pointing, revisit, collection tasking, downlink availability, processing delay, cloud cover where relevant.
- Human/JTAC: local visual observation, target description/mark, and a report that must traverse a valid link.
- Cyber: discovered topology/access rather than geographic sight; effects can alter availability, integrity, or confidence without revealing truth automatically.

Use a broad-phase spatial index followed by exact geometry/terrain checks. Terrain starts with public SRTM/DTED-compatible tiles; NGA describes SRTM coverage over more than 80% of land and its public distribution through USGS ([NGA elevation data](https://earth-info.nga.mil/index.php?action=elevation&dir=elevation)). Cache deterministic terrain samples for tests.

### Communications model

Model each physical/logical link independently: SATCOM, line-of-sight tactical radio, beyond-line-of-sight HF, airborne relay, wired/base network, open internet, and courier/voice reports where scenarios need them. A link has endpoints, frequency/band abstraction, range/LOS, capacity, latency, reliability, crypto/interoperability, emission signature, and jam/cyber susceptibility.

Messages are classified typed payloads (`Order`, `TrackReport`, `FreeText`, `Acknowledgement`, `CollectionRequest`) with size, priority, expiry, originator, and recipient. Routing is store-and-forward over the currently known usable graph. Losing a link does not erase already received knowledge; it makes it stale. Network membership alone never grants global awareness.

## 5. Authority and Roles

### Default Blue chain

The canonical scenario begins with White House Situation Room and President-level authority, then Secretary of Defense, Pentagon/National Military Command Center, combatant command, component command, operations centers/bases, formations, and tactical units. Scenario data owns the actual graph so coalition, delegated, disconnected, and succession cases are testable.

An order must pass all of these checks:

- The player holds the issuing role lease.
- The role may express that order type over the target/echelon.
- A valid authority path or explicit delegation exists.
- Required rules-of-engagement release or two-person confirmation exists.
- The order reaches the recipient through communications.
- The recipient has the capability, inventory, readiness, and local prerequisites.

Failures are observable at the appropriate echelon as rejected, pending confirmation, undeliverable, expired, incapable, or unacknowledged. They are not silently ignored.

### Multiplayer lifecycle

- Game states: `Lobby`, `Loading`, `Running`, `Paused`, `Completed`, `FailedRecovery`.
- Visibility: private/invite, unlisted join code, or public; scenario controls maximum players and role template.
- Role states: available, held, reserved during reconnect grace, or locked by scenario. One player may hold multiple roles only if allowed. Shared staff roles can have multiple seats with explicit permissions.
- Claims use a database transaction and renewable lease. Disconnect reserves the seat for five minutes by default; an administrator can release or transfer it. Reconnect uses a resume token and receives a fresh role snapshot followed by deltas.
- Late join never receives earlier hidden truth. It receives the role's persisted knowledge base, orders, and allowed history.
- Host powers are separate from in-world authority. Creating a game does not make the player President or grant truth view.
- All command attempts, role changes, admin actions, and chat/report traffic are auditable.

## 6. Red AI

Red AI is intentionally basic but must play legally:

1. Consume a Red commander projection containing only Red knowledge.
2. Score a small set of scenario goals: defend area, survive, detect Blue, attack designated target classes, preserve strategic assets, and resupply/retreat.
3. Select from authored task packages and doctrine thresholds.
4. Allocate eligible units with a deterministic utility score and seeded tie-break.
5. Submit the same `PlayerIntent` commands used by humans; authority, communications, capability, sensing, and logistics systems may delay or reject them.
6. Re-plan on a fixed cadence or a significant delivered report, not every frame.

No direct ECS queries for Blue truth are permitted in `sim-ai`. Enforce this with crate boundaries: the AI receives a serializable projection and returns intents. Add adversarial tests that place an undetected Blue unit beside Red and prove Red behavior is unchanged.

## 7. Platform and World Data Program

### Data policy

Treat platform breadth as a pipeline, not a collection of hard-coded structs. Each fact has value, unit, applicability/variant, confidence, source URL/document, retrieval date, license, reviewer, and optional uncertainty range. Preserve conflicting public claims rather than inventing false precision. Jane's can be used only with a valid license and ingestion terms; do not scrape or redistribute proprietary entries. Public official fact sheets, budget documents, technical manuals, and standards are preferred.

The U.S. Navy publishes a large official collection spanning vessels, aircraft, sensors, weapons, EW, communications, and logistics ([Navy fact files](https://www.navy.mil/Resources/Fact-Files/Display/Article/204442/aircraft-carriers/)); use it as a seed index. Use current Air Force fact sheets and budget justification books, service program offices, manufacturer public specifications with lower confidence, and documented open sources for foreign platforms. Every scenario pins a catalog version for replayability.

### Catalog schemas

- `platform`: dimensions, mass/displacement, domain, kinematics, endurance/range, crew, signatures, damage zones, cargo and mounts.
- `sensor`: modality, bands, scan/field-of-regard, nominal detection curves by target class, measurement error, revisit, classification capability.
- `communication`: media/band abstraction, range/propagation mode, throughput, latency, networks, crypto/interoperability and countermeasure response.
- `weapon`: compatible launchers, envelope, seeker/guidance, flight profile, target classes, probability curves, warhead/effects and countermeasures.
- `loadout`: stations/cells/tubes, mutually exclusive stores, fuel/cargo tradeoffs, turnaround/reload constraints.
- `formation`: child units, authorized equipment, echelon, control-area behavior, doctrine and sustainment rates.
- `facility`: runway/berth dimensions, parking/storage, services, throughput, communications and repair.

JSON Schema validates source records; importers normalize to SI units; Rust types validate cross-record references and physical invariants. Generate a source coverage report and block release catalogs containing uncited gameplay-critical fields.

### Aggressive Blue catalog waves

**Wave A, vertical-slice combined force**

- Command/facilities: White House, NMCC/Pentagon, combatant/component operations centers, airbase, naval base, port, ground headquarters, JTAC team.
- Air: F-35A/C, F-15E/EX, F-16C, F/A-18E/F, EA-18G, B-1B, B-2A, B-52H, A-10C, E-2D, E-3, RC-135, E-11A, KC-135R, KC-46A, C-17A, C-130J, P-8A, MQ-9A, RQ-4, HH-60W, MH-60R/S, CH-47F, AH-64E, UH-60M, MV-22B, CH-53K.
- Sea: Ford/Nimitz CVN, Arleigh Burke DDG, Ticonderoga CG for legacy scenarios, Constellation FFG when scenario-appropriate, LCS variants, America/Wasp LHA/LHD, San Antonio LPD, Virginia/Los Angeles SSN, Ohio SSGN/SSBN, Columbia future scenario, Coast Guard National Security Cutter, T-AO and T-AKE logistics ships.
- Land: dismounted squad/JTAC, Stryker variants, M2A4, M1A2 SEP v3, JLTV, M109A7, M270A2, HIMARS, Patriot, THAAD, NASAMS, Avenger, counter-UAS team, AN/TPQ and Sentinel radar families, combat engineers, fuel/ammunition/maintenance units.
- Space: GPS, WGS, AEHF, MUOS, SBIRS/Next-Gen OPIR abstractions, commercial imagery and communications constellations, relay/downlink ground stations.
- Cyber/EW: defensive network operations cell, offensive mission team, SATCOM jammer, tactical communications jammer, GNSS interference, radar deception/noise jamming.
- Weapons/effects: AIM-9X, AIM-120 variants, AGM-88E/G, JASSM family, LRASM, JDAM/SDB families, Tomahawk, SM-2/3/6, ESSM, RAM/SeaRAM, Harpoon/NSM, Mk 46/48/54, Hellfire/JAGM, Javelin, TOW, ATACMS/PrSM, GMLRS, 155 mm families, Patriot interceptors, THAAD interceptor, naval guns and CIWS.

**Wave B, breadth and enablers**

- Air National Guard/Reserve and support aircraft; trainers only where scenario relevant; U-2, E-6B, EC-130/EA-37 mission abstractions, AC-130J, MC-130J, HC-130J, CV-22, MQ-4C, carrier air wing and amphibious air combat elements.
- Amphibious craft, mine warfare, unmanned surface/undersea vehicles, ocean surveillance, salvage, hospital and sealift ships.
- Marine littoral regiment, infantry/armor/cavalry/artillery/air-defense/engineer/logistics battalion templates and their command echelons.
- Fixed and mobile radar, air-defense control, tactical data links, airborne gateways, deployable SATCOM, fiber and public-internet nodes.
- Decoys, countermeasures, sonobuoys, aerial refueling, search and rescue, medevac, battle damage repair, runway repair, munitions handling, and fuel distribution.

**Wave C, scenario depth**

- Coalition Blue variants and interoperability limits.
- Retired platforms needed for historical scenarios and announced platforms only for clearly labeled future scenarios.
- Civil air, maritime, road, and space traffic sufficient to create identification and deconfliction problems.
- Multiple public-source estimates for foreign/Red platform archetypes, but only the fields required for Red AI to sense, move, communicate, attack, defend, and sustain.

Catalog count is not the acceptance metric. A platform is complete only when it has movement, signatures, sensors, communications, compatible loadouts, logistics, damage behavior, source provenance, and at least one scenario test.

### Geographic and orbital sources

- Ingest public GP data as CCSDS OMM rather than building around legacy two-line element limits. CelesTrak documents OMM XML/KVN plus JSON/CSV queries and 9-digit catalog support ([GP data formats](https://celestrak.org/NORAD/documentation/gp-data-formats.php)). Space-Track credentials remain server-side in `.env`; record retrieval epoch and never commit credentials.
- Propagate Earth satellites with a validated SGP4 implementation. Test against published reference vectors and retain each element set's epoch; do not imply precision beyond public GP data.
- Store airport/aerodrome identifiers, runway endpoints, length, width, surface, elevation, status, and source cycle. FAA's 28-day NASR subscription is the authoritative U.S. seed and publishes airport data among its aeronautical products ([FAA aeronautical data](https://www.faa.gov/air_traffic/flight_info/aeronav/aero_data/)). Add global providers only after license review, preserve ICAO/local identifiers separately, and never assume every airfield has an ICAO code.
- Terrain and coastlines are immutable versioned layers; weather is scenario-authored first, then optionally sourced from reproducible public archives.

## 8. Server API and Client

### Initial HTTP API

```text
POST /v1/auth/login
GET  /v1/scenarios
GET  /v1/games?state=running&joinable=true
POST /v1/games
GET  /v1/games/{game_id}
POST /v1/games/{game_id}/join
GET  /v1/games/{game_id}/roles
POST /v1/games/{game_id}/roles/{role_id}/claim
POST /v1/games/{game_id}/roles/{role_id}/release
POST /v1/games/{game_id}/start
POST /v1/games/{game_id}/pause
GET  /v1/games/{game_id}/stream
```

WebSocket client messages include `Resume`, `Intent`, `AcknowledgeFrame`, and `Ping`. Server messages include `RoleSnapshot`, `StateDelta`, `TrackUpdate`, `OrderUpdate`, `Notification`, `Clock`, `RoleLease`, `Error`, and `ResyncRequired`. Every intent has an idempotency key, expected role lease generation, client sequence, and requested execution tick. Every server frame has game/tick/projection sequence.

### Cesium client views

- Lobby/scenario creation and running-game browser.
- Role picker showing echelon, vacancy, sharing policy, and reconnect reservation without exposing hidden force disposition.
- Full-window 3D globe with terrain, time controls, domain filters, track uncertainty regions, sensor footprints when the role may know them, communications status, formation control areas, routes, and orders.
- Selection inspector for role-known identity, confidence, freshness, task, readiness, fuel/ammunition bands, and available legal actions.
- Order composer driven by server-provided action schema and target eligibility; authority and delivery status timeline.
- Track/report workspace for correlation, classification hypotheses, sharing, JTAC reports, and stale/lost contacts.
- Communications/network view containing only nodes and links known to the role.
- Notifications and event log filtered to the role; reconnect/resync, paused, loading, stale, denied, and disconnected states.

Use Cesium primitives/entities with batching and level-of-detail clustering; do not create a React component per simulated object. Keep NATO-style symbology or any third-party icon set behind a license-reviewed adapter.

## 9. Persistence, Recovery, and Replay

- Append player intents, validation results, delivered messages, stochastic outcomes, lifecycle/admin actions, and catalog/scenario version IDs to an ordered event stream.
- Snapshot the minimum complete deterministic world at a fixed tick interval and on pause/shutdown. Include RNG state, queues, knowledge bases, AI state, clocks, and external-data versions.
- Recovery loads the latest compatible snapshot and replays subsequent events. Refuse silent recovery across incompatible simulation schema versions.
- A replay runs the same engine from scenario + seed + event stream. Hash canonical observable state at checkpoints and use it as a determinism regression test.
- Retention and export are deployment policy. Role-filtered after-action review is distinct from controller truth replay.

## 10. Delivery Phases

### Phase 0: Foundation and executable specification

- Create Rust/web workspaces, Compose stack, CI, formatting/linting, schema conventions, architectural decision records, and `.env.example`.
- Implement typed units, IDs, tick clock, seeded RNG, ECS schedule skeleton, catalog/scenario validators, and a headless deterministic test harness.
- Produce a deliberately tiny fictional fixture; no platform research should block engine tests.

**Exit:** `cargo test`, `cargo clippy -- -D warnings`, web typecheck/test/build, and Compose health checks pass; two identical seeded runs produce identical hashes.

### Phase 1: Multiplayer vertical slice

- Auth, scenario listing, game creation, lobby, join, atomic role claims, leases, reconnect, game lifecycle, WebSocket sequencing/resync.
- One Blue command role, one Blue F-35A operator, one airbase, one sensor, one comm path, one weapon, one Red target, and basic Red patrol AI.
- Role-filtered projection and Cesium map with selection and move/engage order flow.

**Exit:** two browser sessions join a running game in different roles; an order is transmitted, acknowledged, executed, and visible only as allowed; a late join and reconnect converge without leaking Red truth.

### Phase 2: Information warfare kernel

- Terrain/Earth LOS, visual/radar/SIGINT measurements, uncertain track fusion and aging.
- Multi-hop store-and-forward networks, link budgets at the selected abstraction, queues, jamming, crypto/interoperability, SATCOM geometry, JTAC report flow.
- Complete authority graph, delegation, rules of engagement, confirmations, failure states, and audit.

**Exit:** scenario tests demonstrate local detection without headquarters knowledge, delayed relayed track delivery, SIGINT uncertainty reduction by triangulation, jammed satellite downlink, unauthorized order rejection, and restored communication without retroactive omniscience.

### Phase 3: Multi-domain simulation

- Air and weapons flight, naval/surface radar, undersea/acoustic, ground control areas and combat, orbital propagation/tasking/downlink, cyber access/effects.
- Logistics, basing, runway/port capacity, refueling/rearming/repair, embarkation, damage and readiness.
- Red goal planner and doctrine packages covering all included Red units.

**Exit:** a 24-hour combined-domain scenario can run faster than real time, pause/recover/replay deterministically, and requires Blue players at strategic, operational, and tactical roles to coordinate across degraded networks.

### Phase 4: Catalog expansion and scenario production

- Complete Waves A and B, airport/runway importer, orbital importer, terrain packaging, source review workflow, coverage reports, and representative 3D/symbol assets.
- Ship three scenarios: small training scenario, regional combined-domain crisis, and large multiplayer campaign.
- Load/performance optimization based on measured scenario sizes.

**Exit:** all Blue release features and platform completeness rules pass; all three scenarios meet budgets and have documented provenance/licensing.

### Phase 5: Hardening and release

- Threat modeling, authorization tests, rate limits, secret handling, database backup/restore, observability, crash recovery, browser accessibility, onboarding, deployment documentation, and soak tests.
- Independent hidden-information audit and scenario balance playtest; label estimates and fictionalized/classified-sensitive abstractions clearly.

**Exit:** release checklist, data licenses, recovery exercise, 24-hour soak, multiplayer playtest, and after-action replay are complete with no known cross-role information leak.

## 11. Verification Strategy

### Unit and property tests

- Geodesy transforms, Earth/radar horizon, terrain occlusion, sensor probability boundaries, covariance growth, routing, authority traversal, inventory conservation, fuel use, weapon envelope, orbital vectors, and catalog validation.
- Property tests for no negative inventory/fuel, stable ID round trips, monotonic message lifecycle, authority cycles rejected, and identical seed/event stream producing identical hash.

### Scenario integration tests

- `detects_unit_when_inside_radar_horizon`
- `does_not_share_contact_across_failed_link`
- `triangulates_elnot_without_revealing_truth_id`
- `jtac_report_reaches_aircraft_through_valid_chain`
- `rejects_order_outside_delegated_authority`
- `late_join_receives_role_knowledge_not_ground_truth`
- `red_ai_cannot_react_to_undetected_blue_unit`
- `satellite_collection_waits_for_swath_and_downlink`
- `aircraft_cannot_launch_from_short_or_damaged_runway`
- `recovery_replays_to_same_world_hash`

### Security tests

Build two projection worlds from the same visible facts but different hidden enemy truth and assert byte-identical role output. Fuzz intent decoding and state-machine transitions. Test horizontal game/role ID substitution, expired leases, duplicate intents, unauthorized admin calls, and malicious scenario/catalog records.

### Initial performance budgets

- 1,000 active platforms/formations, 10,000 orbital/civil background objects, 10,000 tracks, 50 concurrent players, and 20 Hz network delivery while simulation truth advances at 1 Hz.
- Normal tick p95 below 250 ms and no tick above 1 second in the reference regional scenario on documented development hardware.
- Initial role snapshot below 5 MB compressed; normal delta below 100 KB/s per player; reconnect snapshot available within 3 seconds.

Budgets are hypotheses until Phase 1 measurement. Record benchmark hardware and scenario seed in CI artifacts. Optimize broad-phase sensing, terrain caches, projection diffs, and background-object level of detail before weakening simulation semantics.

## 12. First Issue Set

1. Record ADRs for standalone Bevy ECS, fixed-tick determinism, role projections, event log/snapshots, typed units, and catalog provenance.
2. Scaffold the Rust workspace, web app, Compose services, CI, and developer commands.
3. Define `SimEntityId`, `Tick`, typed geospatial/physical units, deterministic RNG, and canonical hashing.
4. Define platform/catalog/scenario JSON Schemas and implement validation with a fictional fixture.
5. Build ordered ECS system sets and ambiguity/determinism tests.
6. Implement game lifecycle tables and transactional role leases.
7. Define versioned HTTP/WebSocket messages with idempotent intent handling and resync.
8. Implement truth/observation/knowledge types and a projection leak test harness.
9. Deliver the Phase 1 F-35/airbase/Red-target vertical slice end to end.
10. Add provenance-aware import adapters for FAA NASR, OMM orbital elements, terrain tiles, and reviewed platform records.

These issues deliberately produce a playable vertical slice before bulk catalog entry. Once the schema and behaviors are exercised end to end, platform research and data entry can proceed in parallel without repeatedly changing the model.
