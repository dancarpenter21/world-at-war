# Verified gameplay checkpoints

These screenshots come from the deterministic Playwright regressions. Training radio settings and target motion are authored scenario data; catalog-dependent tests use an isolated local provider fixture.

## Delivered sensor knowledge

In [Sensor Relay Exercise](../data/scenarios/sensor-relay-exercise.v1.json), CAP Alpha 1 detects a nearby target locally. The command post learns a snapshot only after its track-report packet arrives. The inspector shows the observation age, receipt age, and delivery delay separately. The snapshot keeps its measured position while the target continues moving.

![Delivered track with separate observation and receipt timing](screenshots/sensor-report-delivered.png)

## Executed movement order

The movement controls select a unit, course, and speed. The execution receipt confirms when the delivered and authorized command reaches the simulation. Course changes start a new great-circle arc from the current position, and Stop holds the resulting position.

![Selected aircraft with an executed movement command](screenshots/real-movement-order-executed.png)

## Focused network inspection

The live network opens with connections involving the issuing terminal. The full 48-terminal scenario shows 94 links in this view instead of 2,256. All connections opens the full role-visible graph, and My terminal restores the focused view. Authorized message history remains available independently of the topology focus.

![Issuing-terminal network focus and recorded command delivery](screenshots/real-network-command-delivered.png)

## Map loading and pause

The lobby does not download Cesium until the operational map opens. This regression holds the map module download, pauses the scenario through the still-available host controls, and then allows the map to finish loading. A failed module download offers a reload that restores a current held lease without resuming a paused game.

![Paused scenario while the operational map finishes loading](screenshots/deferred-map-loading.png)

## Reproduce

From `web/`, run `npm run test:e2e` after preparing the pinned sibling c3mesh checkout. The suite builds the Rust server, starts isolated local provider fixtures, and captures fresh screenshots under the ignored `web/test-results/` directory. See [the project README](../README.md) for toolchain and setup commands.
