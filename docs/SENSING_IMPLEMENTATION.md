# Shared time controls and sensing — 2026-10-12

The game header exposes pause/resume and 0.25×, 1×, 2×, 5×, 10×, 30× and 60× pacing.
Only the authenticated host can change pacing. Everyone sees the shared rate,
truth tick, radio-reporting tick when applicable, and elapsed simulation time.
Space Assets retains its independent, explicitly labelled preview clock.

The simulation loop targets **60 frames per real second**. Speed scales game time
per frame: 1× advances 1/60 second, 30× advances half a second and 60× advances one
second. Time uses integer 1/240-second units, so every available speed has an exact
increment. Movement and mission guidance advance per frame; sensor scans, radio
processing, reporting and expiry retain their declared game-second cadences.
Legacy `tick`, observed/received ticks and authored scenario intervals still count
whole game seconds. `frame` counts the new updates, and `elapsed_seconds` includes
the fractional game time. Map viewers target 60 FPS and interpolate received
positions; they do not extrapolate hidden movement. Pause and connection loss stop
interpolation. Interpolation is bounded to one real second and only uses already
received positions.

Games have independent frame deadlines. Slow processing reduces achieved speed;
the server does not skip game time or accumulate unlimited catch-up debt.
Lease/session expiry and archive maintenance remain on wall time. Recovery leaves
games paused and preserves fractional time and the selected rate. New simulation
snapshots use version 3; versions 1 and 2 remain readable. Frame journal inputs
record the selected scale, while historical one-second inputs retain their old
meaning. Older binaries reject version-3 snapshots, so reverting requires the
matching pre-upgrade backup. Role diagnostics report requested and achieved game-time rates. Private
performance traces include sensing duration and candidate/terrain check counts;
candidate counts are not exposed through role projections. The prior sensing load
measurements predate the 60 Hz change and do not certify its durable performance.

## Terrain installation

Regional Joint Campaign and its IFTU derivative require separately installed
terrain. Other exercises use declared Earth-only geometry; **Terrain and SIGINT
Exercise** includes a fictional offline ridge. The scenario picker reports terrain
readiness and disables regional creation when data is missing.

Download and unzip SRTMGL1 V003 HGT tiles for N37W078, N37W077, N37W076,
N38W078, N38W077 and N38W076 using
[NASA Earthdata Search](https://search.earthdata.nasa.gov/search?q=SRTMGL1).
An Earthdata login may be required. Credentials are not required by the importer
or running server. SRTM heights use EGM96 MSL; see the
[USGS source documentation](https://www.usgs.gov/centers/eros/science/usgs-eros-archive-digital-elevation-shuttle-radar-topography-mission-srtm).
Record the download's product/version and licensing alongside the source files.

From the repository root:

```sh
python3 scripts/setup-terrain.py --source /path/to/unzipped-hgt
python3 scripts/setup-terrain.py --verify-only
```

The default output is ignored `data/generated/terrain/regional`. The importer
refuses to overwrite an existing directory. Preserve datasets used by saved games;
use a fresh output directory for another dataset. `TERRAIN_DIR` selects the
installed directory. Compose mounts `data/generated/terrain` read-only and uses
`/app/data/generated/terrain/regional`.

The manifest records checksums, extents, dimensions, source and vertical datum.
Games pin the installed manifest; recovery rejects a different dataset. Tiles are
loaded once and shared within the process. Restart to install another dataset.
Voids and uncovered rays produce unknown visibility, suppressing the observation;
they do not become flat ground. Ground units are placed at sampled terrain height.
Airborne coordinates remain MSL. Terrain intersection uses spherical chord heights
and conservative cell elevation envelopes, sampled at 15 m spacing. This can
block marginal rays on steep slopes; it is a gameplay approximation, not a certified
survey-grade ray tracer. Sensor mounting height is explicit in the versioned model.
The map still uses its existing ellipsoid terrain presentation.

## Sensing, knowledge and controls

`data/sensing/training-model.v1.json` and `platform-profiles.v1.json` contain
fictional performance assumptions. Visual/EO and IR include visibility, clouds,
daylight or thermal contrast. Radar includes horizon/terrain, scan revisit,
field of regard, aspect/signature classes, look-down clutter, rain attenuation,
emission state and a simplified signal/interference model with close-range
burn-through. Radar jamming uses the exercise's authored band and power abstraction.
Solar elevation is a deterministic low-order seasonal/daylight approximation.
Weather is scenario-authored or set by an explicitly granted controller, not a live
weather service. Models do not assert real platform compatibility or performance.

Noise uses the scenario seed and stable observation keys, independent of browser
count. Speed changes the per-frame integration interval; comparisons must use
the same timed inputs and speed schedule. Position fixes carry horizontal covariance,
modalities and evidence IDs. Terminal-local constant-velocity filters associate
measurements using geometry, uncertainty and compatible classification evidence.
Candidates pass a Mahalanobis gate and are ranked by normalized Gaussian
likelihood so broad estimates do not outrank precise compatible observations;
ambiguous associations remain separate. Reports use conservative covariance
intersection, source watermarks and observation deduplication. Reports carry a rolling eight-observation evidence
window; private deduplication retains the full live window and forwarded reports
always use conservative covariance intersection. Campaign reports and upgraded
explicit subscriptions are paced at least ten simulation seconds apart. Each
subscription waits for its current reliable batch to settle before sampling the
latest knowledge, bounding report backlog on constrained links. Internal campaign
knowledge reports likewise wait for preceding traffic on their directed route;
queued control traffic suppresses starting a fresh internal knowledge batch. Track prediction,
confidence decay and 120-second expiry use simulation time, including in engagement
eligibility. Delayed reports preserve observation and receipt times and cannot
rewind newer source reports or extrapolate an estimate twice.

Passive receivers obtain bearings and fictional emitter-class matches. Independent,
compatible receivers can form a weighted position fix; nearly parallel, inconsistent
or ambiguous bearings remain regions. Several simultaneous same-class emissions
at a receiver suppress triangulation rather than selecting an arbitrary bearing. Bearing-only contacts cannot be engaged.
The map shows uncertainty ellipses and passive bearing wedges. Classification is
still a simplified evidence/range threshold, not a full target-recognition model.

**Radar emissions** uses the existing authenticated authority/order workflow.
On/off commands wait for transport delivery and execution. Lost responses can retry
the original intent ID. Receipts distinguish execution from confirmation; remote
status is learned from delivered friendly reports. Radar off leaves passive sensors
available. Friendly emitter observations are filtered by the authored training
recognition rules. Ordinary roles get local weather and explicitly delivered weather reports.

The Game Controller workspace includes weather editing and a map preview for
geographic regions. Hosting alone does not authorize edits; hosts must grant
and claim a controller seat. Game Monitors can inspect weather but cannot edit it.
Changes use revision checks, preserve actor/role/conditions in an intervention
record and commit before activation at the next truth tick. Paused games remain
paused. Region precedence is highest priority, then lexically lowest ID.
Richer training reports use the existing fragmentation/reassembly engine. The
three radio exercises use separately authored upgrade limits (9.6 kbit/s, 4 KiB
MTU, 16 queued packets and 64 KiB per queue) and retain their blackout behavior.
Training combat gains a finite post-mission reporting window so acknowledgements
can settle after impact. Radio migration preserves packet IDs, queued/scheduled
events and channel states; future admissions and serialization use the new limits.
The adapter checks the pinned c3mesh snapshot wire version.

The editor accepts a baseline and up to 64 JSON-authored regions:

```json
{"id":"cloud-bank","center":{"latitude_deg":38.1,"longitude_deg":-76.5,"altitude_m":0},"radius_m":10000,"priority":1,"start_tick":0,"end_tick":null,"conditions":{"visibility_m":5000,"cloud_base_m":1000,"cloud_top_m":3000,"cloud_opacity":0.8,"precipitation":0.3,"thermal_contrast":0.6}}
```

Weather attenuation currently combines conditions at the observing and target
positions; it does not integrate atmospheric layers along an entire ray.
No radio-terrain propagation, weapon-terrain collision, acoustic sensing or
terrain-following flight is added by this milestone.

## APIs and migration

- `PUT /v1/games/{id}/time-scale`: host only, `{ "time_scale": 5 }`.
- `GET /v1/games/{id}/environment?role_id=...&lease_generation=...`: active granted observer.
- `PUT /v1/games/{id}/roles/{role}/environment`: active granted controller;
  `{ "lease_generation": ..., "expected_revision": ..., "baseline": {...}, "regions": [...] }`.
- Existing intent API adds `{"SetRadarEmission":{"emitting":false}}`.
- Game summaries add clock fields; scenario summaries add terrain readiness.
  Role projections add optional sensing data and track evidence. Existing positional
  track fields remain. Bearings are separate from tracks.

Existing games need an explicit stopped-server migration before the new server
accepts their upgraded scenarios. Back up PostgreSQL and preserve the old binary,
source pin and terrain dataset before deployment. Keep the server stopped while
running the same binary with the intended `DATABASE_URL` and `TERRAIN_DIR`:

```sh
target/release/world-at-war-server --migrate-sensing --dry-run
target/release/world-at-war-server --migrate-sensing
```

The runtime fence rejects another active server. The dry run verifies recovered
state and predecessor scenario/catalog/dependency pins and prepares all upgrades
without changing game records. Successful migration appends version-2 checkpoints
and updates scenario metadata in one transaction across the games. Historical
journal/audit rows are retained, along with memberships, accepted intents, pending
transport and existing scoped track IDs. Each migration records predecessor and
successor scenario checksums and its simulation tick. Games return paused. Unknown
predecessors, missing required terrain or commit errors abort the upgrade.
The legacy constructors/codec remain available to verify old journal segments.

Rollback requires restoring the pre-migration database backup and matching old
binary; the old server does not understand version-2 snapshots. These commands do
not implement interactive historical replay or distributed authority.

## Verification

Use workspace/frontend/browser checks in AGENTS.md. Real PostgreSQL migration
fixtures are part of the ignored server suite and require an isolated
`TEST_DATABASE_URL` with CREATEDB permission. Browser fixtures generate labelled,
flat synthetic tiles in their owned temporary runtime, never use live terrain
credentials and never replace development data.

`--benchmark --sensing` opts the offline release benchmark into upgraded sensing
and requires regional terrain. The default benchmark retains the legacy workload
for canonical-hash comparisons. Short full-server runs use upgraded scenarios.
Real-terrain reference budgets and long-term stability require separate acceptance
measurements. Long soaks are never started without an explicit user request.

Final functional checks pass; regional durable load acceptance remains open. See
[verification results and synthetic-terrain measurement limits](performance/2026-10-12-sensing-results.md).
