# Terrain-aware radio delivery

New Regional Joint Campaign games, their IFTU derivative, and Global Crisis use
explicit line-of-sight propagation on eligible ordinary radio links. Regional
reuses its required, checksummed sensing terrain. Global uses Earth geometry by
default; an authored sensing configuration with terrain also enables terrain
checks. Wired links and space, undersea and dynamic weapon links retain their
existing models. Existing saved games retain their original rules.

## Play the offline exercise

Select **Ridge Relay Exercise**, create a game, claim **Exercise Commander**, and
start. Send a movement order to **Ridge sensor aircraft** during the first few
seconds. A second player can claim **Pilot, CAP Alpha 1** to see the nearby contact
locally. Headquarters initially has no contact report: a fictional ridge blocks
the direct radio path.

The scripted relay climbs into position during the first eight game seconds.
Its two clear hops restore delivery; the direct path remains blocked. Headquarters
receives the original observation, and the queued order reaches the sensor
platform. Execution confirmation arrives separately over the return route.
Commands, reports and replies compete on the same configured radio. Pausing
freezes movement and radio time; resuming preserves pending traffic.

Open **Network**, inspect headquarters, then select its link to the sensor
platform. **Geometry** shows **Terrain blocked**, independently of the interference
percentage. The operational inspector and authorized White Cell truth inspector
show the same geometry vocabulary. Unknown terrain means unavailable coverage,
not receiver jamming or assumed flat ground. The scripted relay has no commander
movement policy and is excluded from that command scope.

## Configuration and interfaces

A communication link can add an optional `propagation` object:

```json
{
  "version": 1,
  "terrain": true,
  "source_antenna_height_m": 10.0,
  "destination_antenna_height_m": 2.0,
  "max_range_m": 550000.0
}
```

Omission preserves legacy behavior. Configuration requires a radio channel,
version 1, finite nonnegative mounting heights and a finite positive range.
Campaign defaults are fictional: 10 m for ground terminals, 20 m for surface
vessels and 2 m for aircraft, with the existing 550 km horizontal range cap.
Antenna height is added to platform MSL altitude, not to terrain a second time.

Initialization occurs after scenario sensing and ground positioning. Subsequent
checks happen once per game second, including shared speed changes. The latest
result is cached per link until endpoints, settings or terrain identity change.
Earth obstruction, range and the existing conservative terrain visibility model
control channel availability; required but uncovered terrain fails closed.
Geometry clearance restores the authored channel state, including degradation
or a deliberately severed channel. Traffic through the previous boundary is
processed before changing state. Already-started transmissions retain c3mesh's
existing semantics.

Existing state, network and truth REST/WebSocket projections add optional
`geometry_status`: `clear`, `earth_blocked`, `terrain_blocked`, `terrain_unknown`,
or `out_of_range`. `available` remains the combined channel result; `jammed`
continues to describe interference. Endpoint and role authorization filters apply
before geometry is exposed. Absence preserves older client inputs and denotes
links without this model.

Opted-in scenarios send nonlocal ordinary C2 messages through the existing reliable
fragmented transport. Priority, traffic class, flow identity, expiry and forwarding
budget travel with fragments. Relay routes use only configured usable directed
links; physical proximity does not create compatibility. Retries retain their
existing three-attempt/five-second limits. Restoration after expiry or exhausted
attempts cannot resurrect a message. A later reporting attempt samples a new
observation. Message timing starts with the first actual fragment transmission; delivery
requires complete reassembly. Reliable track-report sender views retain their
acknowledgement boundary, and received execution replies remain inspectable after
transport acknowledgement. Unacknowledged messages are recognized by the client.
Transport acknowledgement and execution confirmation are separate
messages; neither is synthesized from successful forward delivery.

## Saves and limits

Optional propagation settings and the last radio-boundary statuses are saved by
the existing snapshot system. Derived caches are rebuilt lazily on the next radio
boundary, preserving fractional-frame recovery. Recognized predecessor scenarios
remain recoverable without adopting new radio rules. No database migration or
new persistence architecture is introduced.

The ridge is synthetic and the radio parameters are training estimates. This
binary model does not implement diffraction, refraction, Fresnel clearance,
link budgets or weather-dependent RF propagation. Synthetic and flat test terrain
do not establish real-SRTM accuracy. The operational map keeps its existing
ellipsoid terrain presentation; network colors and inspectors communicate the
simulation's obstruction result.

The offline release benchmark accepts `--radio`, which also enables sensing,
for short regional/global propagation measurements. Regional needs `TERRAIN_DIR`;
use clearly labelled synthetic data only for software measurements. This option
does not run the unattended soak runner.

## Validation checkpoint

265 fast Rust tests, 19 isolated PostgreSQL tests, 48 frontend tests and all 66
Chromium cases pass, including the dedicated-database crash/recovery browser case.
Formatting, strict workspace Clippy, dependency-pin verification, browser
TypeScript checking, Rust 1.89 workspace compilation and production
frontend/release-server builds pass. The fast
performance harness passes 22 cases with its opt-in Docker cleanup case skipped.

[Verified screenshots](gameplay-checkpoints.md#ridge-relay-radio) show blocked and
restored radio on desktop and phone. [Short measurements](performance/2026-10-13-radio-results.md)
use labelled synthetic flat regional terrain and Earth-only Global geometry;
repeated role-projection hashes agree. No long soak, live catalog request,
real-SRTM reference acceptance or development-database migration was run.
