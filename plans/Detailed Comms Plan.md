 # Detailed C2 Network Simulation for Existing Scenarios

  ## Summary

  Implement a packet-level, deterministic communications layer across the sibling c3mesh library and World at War.
  The delivery will equip every entity in Global Crisis and Jammed Flight Test, route all commands, authority
  decisions, acknowledgements, and shared sensor reports through simulated terminals, and add role-filtered
  network graph and map views.

  The committed message pack will be a clearly labeled public-safe subset. Authorized MIL-STD-6016/6040 packs can
  be loaded externally but will not be committed because current ASSIST records restrict detailed distribution.
  MIL-STD-6016 record (https://quicksearch.dla.mil/qsDocDetails.aspx?ident_number=123964), MIL-STD-6040 record
  (https://quicksearch.dla.mil/qsDocDetails.aspx?ident_number=214270).

  ## Key Implementation Changes

  ### 1. Extend c3mesh as the generic packet engine

  - Add bounded ingress/egress queues, router processing rates/delays, per-link MTUs, shared-medium capacity, and
    deterministic queue-overflow and channel-loss behavior.

  - Add configurable IP-style hop limits, packet expiry, traffic class, flow/message/fragment IDs, content tags,
    classification metadata, and wire-size overhead.

  - Provide FIFO, strict-priority, and weighted-fair queue disciplines. Expose a registered QueueDiscipline hook
    that selects packets from metadata without introducing C2-specific types into c3mesh.

  - Add deterministic seeded packet loss and new drop reasons for queue overflow, processor overload, expiry, MTU
    mismatch, and configured channel loss.

  - Add advance_to(SimTime), runtime route updates, and runtime channel-condition updates so World at War advances
    networking only to each simulation-tick boundary and leaves later packets queued.

  - Allow compatible point-to-point channels to reference a shared medium so pairwise tactical links cannot
    multiply one radio network’s capacity.

  - Preserve current APIs through defaults: existing configurations retain their present unbounded FIFO behavior,
    while World at War always supplies explicit bounded configurations.

  ### 2. Add modular communications catalogs and runtime

  - Introduce sim-comms for C2 message encoding, fragmentation/reassembly, routing, terminal selection, transport
    retry, lifecycle tracking, topology knowledge, and network projections. Keep physical packet scheduling in
    c3mesh, platform facts in sim-catalog, and gameplay effects in sim-core.

  - Load versioned YAML under data/communications for:
      - Radio, computer, modem, router, antenna, cable, gateway, and SATCOM-terminal profiles.
      - Operating modes with frequency band, bandwidth, nominal/user data rate, range/link-budget parameters, MTU,
        protocol/interoperability tags, and crypto/classification capability.

      - Platform equipment assignments, quantities, effective dates, field-level provenance, and confidence.
      - Infrastructure, traffic-generator, queue-policy, and public-safe message-definition profiles.

  - Add JSON Schemas and a validator/coverage-report CLI. Startup fails on malformed catalogs, unresolved
    references, impossible rates/bands, duplicate IDs, or wireless modes lacking both a frequency band and data-
    rate model.

  - Rank facts as official, manufacturer, corroborated public, or estimate. Estimates require a written rationale
    and source archetype; exact values are never invented silently.

  - Pin each game to the normalized communications-catalog checksum, message-pack checksum, scenario date, network
    policy ID, and deterministic seed.

  - Populate every existing entity:
      - Facilities join reliable regional backbones through campus routers and command terminals.
      - Island and overseas facilities receive diverse submarine-cable and SATCOM paths.
      - Ships, submarines, aircraft, formations, cyber cells, and space-support cells receive platform-specific
        profiles where public data exists.

      - Generic Red formations and undisclosed parameters use visibly marked conservative archetype estimates.
      - Every entity has at least one operational computer/terminal path at scenario start.

  - Represent backbone routers, cable landings, gateways, and satellite relays as network infrastructure nodes
    that may exist without becoming combat-map entities.

  - Generate wireless edges from compatible modes, network membership, crypto state, geometry, Earth-horizon LOS,
    range/link budget, and interference. Effective data rate comes from the selected operating mode and current
    conditions, not carrier frequency alone.

  - Author fixed wired/cable/SATCOM infrastructure paths explicitly. Different sides may share physical Internet
    infrastructure but remain isolated by routing, network membership, and crypto compatibility.

  - Replace Global Crisis’s dummy sink-only topology and retain Jammed Flight as the directional-jamming
    regression. Bump their scenario IDs/versions because active games are ephemeral and no migration is required.

  ### 3. Route gameplay through real simulated messages

  - Define a versioned C2Message containing originator role/entity/terminal, recipients, standard/profile IDs,
    message type, structured fields, canonical rendered text, exact encoded bytes, classification, priority,
    authority claim, creation/expiry time, and delivery mode.

  - Ship public-safe definitions for movement orders, engagement orders, authority requests/decisions,
    acknowledgements, track reports, free text, network management, and heartbeat/background traffic. They must
    not claim full MIL-STD conformance.

  - Add a definition-pack interface for externally supplied authorized J-series/USMTF definitions. External packs
    live in an ignored configurable directory and are validated/checksummed before use.

  - Fragment encoded messages into packets using the selected transport profile; destination terminals reassemble
    and validate them. Commands and authority traffic use acknowledged delivery with bounded retries/backoff,
    while track and background reports default to best effort and may be coalesced.

  - Remove AlwaysReachable. Submission validates that the role has authority and a compatible local terminal, then
    queues a message. It does not queue an ECS intent directly.

  - Advance authority workflows only when request or decision messages reach the appropriate role’s terminal.
    Final commands become AuthorizedIntents only after complete delivery and authority-claim validation at the
    target.

  - Send application acknowledgements back through the network. A command may execute while its issuer remains
    unaware if the acknowledgement is lost.

  - Treat message and intent IDs as idempotency keys so retries or duplicate fragments cannot execute a command
    twice.

  - Keep local sensor observations local. Shared track knowledge changes only when a track-report message is
    delivered.

  - Queue route-less messages until a route appears or they expire; report queued, in-transit, delivered,
    acknowledged, dropped, expired, and retrying states instead of immediately returning “blocked comms.”

  - Pin information-management policies per game. Content-aware policies classify the complete message at
    authorized endpoints or trusted gateways; routers unable to decrypt may use only exposed metadata.

  ### 4. Server, persistence, and UI contracts

  - Add role network-access privileges: local/known topology, side topology, ground-truth topology, monitored
    networks, readable classifications, and content-monitoring authorization. Add an Exercise Controller role to
    each scenario for ground-truth testing.

  - Add APIs:
      - GET /v1/games/{id}/network for a role-filtered NetworkProjection.
      - WS /v1/games/{id}/network/stream for sequenced node/link/traffic deltas and resynchronization.
      - GET /v1/games/{id}/network/events for cursor-paginated history and filters.
      - GET /v1/games/{id}/network/messages/{message_id} for authorized message details.
      - Game creation fields for seed and advertised scenario-compatible policy ID.

  - Define network projections with device/infrastructure nodes, physical/logical links, current capacity and
    throughput, queue occupancy, latency, utilization, loss/drop counters, jamming, staleness, and recent flow
    summaries.

  - Change intent/authority submission outcomes to return the created message and lifecycle IDs. Preserve the old
    flat communication_links projection field for one compatibility version, populated only with role-visible
    aggregates.

  - Store the full run’s message and packet events in an append-only segmented event store under an ignored
    runtime directory. Store message content once and reference it from packet events; keep only a live tail and
    aggregate counters in memory. Pause the game and expose an operational error if durable event writing fails.

  - Add a lazy full-screen Network workspace using the existing XYFlow/ELK stack:
      - Entity/infrastructure aggregate view with expandable physical-device detail.
      - Animated edges showing current traffic, width by throughput, and color by health/utilization.
      - Filters for side, network, medium, band, status, message class, and classification.
      - Inspectors for equipment/provenance, queues, link metrics, routes, drops, and authorized decoded message
        text.

      - Redacted metadata in place of content when the viewing role lacks access.

  - Add a map-filter toggle for the network overlay, off by default. Render only role-visible positioned nodes and
    links, using utilization, jamming, and failure styling consistent with the graph.

  - Update Docker images to include the committed catalogs and mount a writable network-event directory. Document
    catalog, authorized-pack, and event-store environment variables.

  - c3mesh: bounded saturation, FIFO ordering, priority/WFQ behavior, custom scheduler hooks, processor overload,
    shared-medium contention, MTU/TTL/expiry drops, deterministic seeded loss, tick-boundary advancement, route
    changes, and in-flight channel-condition semantics.

  - Catalog/scenarios: schema validation, provenance/confidence enforcement, checksum stability, and a coverage
    assertion that all 66 existing units resolve to at least one terminal and every wireless mode has valid band/
    rate data.

  - Simulation: commands cannot execute before complete authorized delivery; jammed, saturated, expired, or
    incompatible paths prevent delivery; recovered links deliver queued traffic; duplicate/retried commands
    execute once; acknowledgements can fail independently.

  - Information flow: local contacts remain local, relayed track reports arrive after measured delay, full queues
    drop low-priority traffic first under the selected test policy, and policy selection produces deterministic
    repeatable outcomes.

  - Security/projections: normal roles cannot infer hidden nodes, links, flows, or content; authorized monitors
    see allowed content; controller projections contain ground truth; stale known topology remains marked rather
    than disappearing.

  - Server/storage: lease checks, cursor pagination, stream sequence/resync, content redaction, pinned policy/seed
    validation, full event retrieval, and event-writer failure handling.

  - Frontend/E2E: network graph updates without recreating nodes, traffic/link inspectors work, the map overlay
    toggles cleanly, Jammed Flight visibly degrades and recovers directionally, and controller versus normal-role
    views differ correctly.

  - Run cargo fmt --check, cargo clippy --workspace --all-targets, and all tests in both repositories, followed by
    frontend tests/build and Docker Compose validation. Benchmark Global Crisis against the existing target of
    normal tick p95 below 250 ms.

  ## Assumptions and Defaults

  - Initial delivery covers the reusable engine and all entities in the two current scenarios.
  - Wireless fidelity stops at deterministic packet/link, shared-medium, LOS/link-budget, and interference
    behavior; waveform modulation and Link-16 time-slot simulation remain extension points.

  - The committed message pack is public-safe and non-conformant; restricted definition material is accepted only
    through an authorized external pack.

  - Unknown equipment and parameters use marked conservative estimates rather than leaving entities disconnected.
  - Normal players see role-known topology; only scenario-granted controllers see full ground truth. Full message
    content is limited to endpoints, authorized monitors, and controllers.

  - Queue/information policies are immutable once a game starts. Comparing technologies means running the same
    scenario, seed, catalog, and traffic with different pinned policy IDs.

  - Full event history is guaranteed for an active run, but complete game crash recovery remains outside this
    delivery because the rest of the game runtime is currently memory-resident.
