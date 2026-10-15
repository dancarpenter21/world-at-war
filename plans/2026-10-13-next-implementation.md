# Feature status and next implementation step — 2026-10-13

Current update — 2026-10-14: the first goal-directed defensive AI and external
agent/OpenRouter adapter are implemented; see [AI behavior and limits](../docs/AI_IMPLEMENTATION.md).
The fictional joint sustainment loop is implemented; real-platform rollout remains
blocked by the documented [source gaps](../docs/LOGISTICS_IMPLEMENTATION.md).
The assessment below records the earlier checkpoint and does not supersede those
new implementations. Remaining domain loops and interactive review follow.

Status: feature assessment initially reviewed against `ca71e40`, updated after
the White Cell and terrain-aware radio increments. Implementation evidence lives
in their checkpoint documents; the remaining release work below is planned.
Newest checkpoints supersede older statements that persistence or sensing is absent.

## Are all game features implemented?

No. The repository implements a playable multiplayer command/campaign prototype,
but does not fulfill the release contract in IMPLEMENTATION_PLAN.md sections 1
and 10. Presence of a scenario, catalog viewer or domain enum does not establish
the corresponding domain's complete gameplay loop.

| Area | Implemented foundation | Remaining release work |
| --- | --- | --- |
| Multiplayer and command | Sessions/accounts, roles/leases, authority workflows, delivered commands, execution acknowledgements, collaborative campaign plans and airspace orders | Broader controller inspection and interventions beyond weather; release hardening and playtest evidence |
| White Cell referees | Individual referee seats, approved joining/transfers, omniscient truth/plan/message/role inspection, shared notes, lifecycle/speed and weather controls | Exercise injects, outcome overrides, richer adjudication and interactive review |
| Information warfare | Terrain/Earth sensing, visual/IR/radar models, uncertain tracks, passive bearings/fixes, delayed reports, network queues/jamming, terrain-aware ordinary radio, metadata audit | Richer target recognition, full information-management fidelity; real-terrain reference acceptance |
| Combat and domains | Air mission execution, simplified weapons/damage, fictional IFTU, public orbital propagation/viewing and authority-routed satellite requests | Complete naval, undersea/acoustic, ground and cyber gameplay; orbital collection/tasking/downlink effects |
| Logistics and basing | Training fuel/ammunition abstractions and airport/runway catalog | Resupply, refueling/rearming/repair, operational runway/port capacity, embarkation and readiness |
| Red AI | Projection-constrained airborne patrol orders with delivery/retry handling | Goal scoring, doctrine/task packages, allocation, reactive replanning and coverage of remaining domains |
| Persistence and review | PostgreSQL state, snapshots, deterministic restart recovery, role-filtered communications metadata history and optional retention | Interactive historical replay/after-action review and export; broader recovery/release exercises |
| Data and performance | Authored scenarios, communications/airport/orbital catalogs, short-run measurement tools and 60 Hz scheduling | Release catalog waves, remaining scenario coverage, responsive gameplay, roadmap-scale/browser validation; durable throughput optimization deferred |

Evidence: [implemented behavior](../README.md), [release phases](../IMPLEMENTATION_PLAN.md#10-delivery-phases),
[sensing limits](../docs/SENSING_IMPLEMENTATION.md), [patrol planner](../crates/sim-ai/src/lib.rs),
and [committed-frame measurements](../docs/performance/2026-10-12-frame-results.md).
This is a feature-area assessment, not certification of every phase exit criterion.

## Priority correction: gameplay before stronger durability

The user has explicitly reduced the priority of durability. This supersedes the
previous recommendation to optimize synchronous per-frame persistence first and
the older durable-performance prerequisites in IMPLEMENTATION_PLAN.md.

- Build missing gameplay features next. Durable 60 Hz, comprehensive event
  journaling and exact crash replay are not prerequisites for feature development.
- Keep existing optional persistence/recovery available. Do not invest in the
  proposed committed-state cache or the full durable benchmark matrix now.
- Prefer a simpler save/load design if persistence work becomes necessary:
  periodic or explicit whole-game saves, with recovery to the last completed save.
  Save cadence and acceptable lost progress need definition when that work is
  scoped; this plan does not silently change current runtime guarantees.
- Keep server authority, role ownership, hidden-information boundaries and
  command delivery/idempotency correct during play. These gameplay requirements
  are separate from preserving every frame after a crash.
- Validate gameplay performance in the existing volatile mode. If the active
  deployment requires PostgreSQL and persistence blocks play, scope a small
  save/load simplification before adding more durability optimization machinery.

## Terrain-aware radio delivery — implemented checkpoint

The terrain-aware radio increment is implemented; see [behavior and limits](../docs/RADIO_PROPAGATION_IMPLEMENTATION.md). The implementation outline below records its intended scope. Goal-directed Red AI is the next recommended increment.

Make terrain affect whether orders and reports can actually reach their
recipients. The playable result is an offline ridge exercise in which a direct
radio path fails, a compatible relay restores delivery, and recipients learn
about contacts only when reports arrive.

### Implementation sequence

1. **Define a small fictional radio fixture.** Reuse the deterministic ridge
   terrain and add a sensor, headquarters terminal and relay. Author radio modes,
   antenna heights and configured links as data. Cover direct blockage, relay
   availability and movement that restores the direct path.
2. **Apply terrain to eligible radio links.** Extend communication geometry in
   `crates/sim-core/src/lib.rs` using `sim-geo` terrain visibility and the existing
   sensing terrain resource. Apply Earth/terrain obstruction only to configured
   line-of-sight radio modes; retain existing behavior for other media. Use
   antenna height above the platform's MSL position. Unknown terrain blocks an
   eligible path in terrain-required scenarios; scenarios without terrain retain
   their authored geometry. Treat this as a binary gameplay approximation, not a
   full RF propagation model.
3. **Use the existing transport and routing.** Feed availability into the current
   c3mesh channel mechanism. Reuse usable-route selection and fragmented reliable
   delivery in `crates/sim-core/src/transport.rs`. Preserve equipment/network
   compatibility, bandwidth, queues, retry/expiry, independent acknowledgements
   and execution after complete delivery. Recompute at the existing radio cadence,
   not at every animation frame; reuse terrain results for unchanged endpoints.
4. **Explain the result in the network UI.** Show role-visible link state and a
   terrain-blocked or terrain-unknown reason where supported by the projection.
   Avoid exposing remote terrain failures or relay identities that the role is
   not authorized to inspect. Reuse the current network workspace and map preview.
5. **Ship the exercise and focused validation.** Add deterministic scenario tests
   and one browser flow for blocked command/report delivery followed by relay
   restoration. Document the abstraction and add a verified gameplay screenshot.

Primary files: `crates/sim-core/src/lib.rs`, `transport.rs`, `sensing.rs`,
`crates/sim-geo/src/terrain.rs`, scenario fixtures/loaders, and the existing network
projection and `web/src/NetworkWorkspace.tsx`. Inspect existing channel-state
support before choosing whether a new public reason field is necessary.

### Acceptance criteria

- A ridge blocks an otherwise in-range direct radio path; an unobstructed path
  works. Antenna height and endpoint movement affect the result predictably.
- A compatible configured relay permits delivery; incompatible or blocked relay
  paths do not. A broken return path cannot produce a sender acknowledgement.
- A locally detected contact remains unknown to headquarters until its report
  arrives. Reconnection delivers queued valid traffic according to existing
  expiry rules and never reveals intervening unreported truth.
- A command cannot execute before complete delivery and cannot execute twice
  because of retransmission or route restoration.
- Terrain voids, missing coverage and scenario geometry defaults have explicit
  deterministic tests. Existing training scenarios retain their behavior.
- Network views communicate authorized failure states; tests check that they do
  not expose hidden endpoints or remote knowledge.
- Existing save/load remains compatible through the project's normal versioning
  rules. Do not expand the persistence architecture for this feature.

Run focused Rust/scenario and frontend tests during implementation, then the
repository formatting, lint, Rust, frontend build/test and relevant browser checks.
Use a short offline exercise to check responsiveness with radio terrain enabled.
Synthetic fixture success does not establish real-SRTM accuracy. Real-terrain
validation can follow when operator data is available. No long soak or durable
performance matrix is required for this increment.

## White Cell referee team — core increment implemented

The core team/referee increment is implemented, including White Cell hosting,
approved joining of running/paused games, human Red selection, private notes and
shared lifecycle/speed controls. See [implementation and limits](../docs/WHITE_CELL_IMPLEMENTATION.md).
New exercise injects, outcome adjustments and interactive replay remain follow-up
work. Existing optional saves suffice; stronger durability is not a prerequisite.

## Other following gameplay increments

1. Goal-directed Red AI: defend/patrol/intercept decisions using delivered role
   knowledge, deterministic task selection and the existing legal order flow.
2. Logistics and basing: finite supplies, refuel/rearm/repair and a small playable
   sortie-turnaround scenario.
3. Remaining naval, undersea, ground, orbital collection and cyber domain loops,
   each with a bounded scenario and observable player decisions.
4. Catalog/scenario expansion and interactive after-action review as useful to
   those loops. Additional persistence work follows an actual save/load need.
