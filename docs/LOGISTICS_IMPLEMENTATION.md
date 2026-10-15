# Joint sustainment mechanics and aircraft fuel coverage

The **Joint Sustainment Exercise** provides an offline, explicitly fictional
logistics loop: finite stocks, automatic servicing, physical cargo missions,
moving-carrier recovery, underway replenishment, and boom/drogue aerial refueling.
The exercise parameters are testable game rules, not specifications for real
platforms. It needs no orbital catalog, provider credentials or terrain download.

New Regional games expose missing aircraft logistics evidence. They do **not**
enable aircraft fuel consumption, refueling or basing through invented defaults.
The named Regional aircraft remain under their preceding campaign rules until
suitable logistics profiles are supplied. This checkpoint does not complete the
planned real-platform Regional logistics rollout.

## Play the exercise

Create **Joint Sustainment Exercise**, claim **Exercise Commander**, and start.
**Logistics and basing** appears in the command panel. Each individual platform or
facility also has an operator seat for multiplayer play. The commander initially
knows headquarters' stock; remote resource reports arrive over the radio.
Their observation and receipt times remain visible after pausing or reconnecting.

For aerial refueling:

1. Select **Exercise Joint Tanker**, choose **Boom**, and establish support.
2. Wait for the execution receipt. Select **Exercise Boom Fighter**, choose the
   tanker as donor, request 100 kg and send the fuel-transfer task.
3. Watch rendezvous, connection, transfer and separation in received reports.
   The fictional receiver accepts 10 kg/s; the donor's 20 kg/s equipment ceiling
   does not make that receiver accept fuel faster. Both aircraft keep burning fuel.
4. After separation, change the tanker to **Probe and drogue** support and use
   **Exercise Carrier Fighter** as receiver. Changing or cancelling support
   interrupts an existing session and preserves completed transfers.

For physical replenishment, select **Exercise Cargo Aircraft**, choose
**Forward Airfield** as cargo source and headquarters as destination. Dispatch
fuel, stores or repair supplies. The depot has a finite 1,000 kg fuel allocation;
headquarters and the carrier each begin with only 40 kg. Stocks are reserved when
the task reaches the transport, loaded at source, carried and unloaded at the
destination. They never appear remotely just because dispatch succeeded.

Select **Exercise Supply Ship** to deliver cargo from **Exercise Port** into
**Exercise Carrier**. The ship connects underway equipment, transfers into carrier
stocks, then separates; it is not parked aboard the carrier. To refuel the escort,
establish **Underway replenishment** support on the supply ship and send a matching
receiver task from **Exercise Surface Escort**. This uses the ship's supply fuel
stock rather than its propulsion tank.

Return an aircraft to a compatible airfield or carrier. Parking/berth capacity,
movement slots and service slots constrain arrivals and departures. Refueling,
rearming and repair start automatically in arrival order. Depleted supplies leave
a visible incomplete turnaround. A unit may depart early with minimum operating
fuel and sufficient health; destroyed units cannot be repaired. Replenish the base
and launch another sortie to exercise the complete loop.

## Model and interfaces

`Scenario.logistics` is optional. Version 1 configuration supplies unit profiles,
facilities, explicit report subscriptions and unavailable capability records.
Scenario construction validates references, same-side home assignments, capacities,
finite rates, operating envelopes and evidence metadata before starting the game.

`OrderKind::Logistics { command }` uses existing movement authority and the normal
lease, approval, communications, delivered execution and acknowledgement lifecycle.
Commands are `return`, `depart`, `cancel`, `support`, `refuel` and `cargo`.
Mission tasks may add `home_facility_id`; otherwise the unit's configured home
applies. Mission return enters facility recovery and completes upon arrival,
separately from turnaround. Units awaiting logistics readiness cannot execute
mission movement or weapons release.
Acceptance starts a task; completion is a subsequent local or delivered report.
Duplicate intent IDs cannot repeat reservations or transfers. Source and destination
compatibility and inventory are rechecked by the receiving executor.

Role state adds optional `logistics`; authorized truth inspection adds its own
logistics projection. Resource knowledge is terminal-local or report-delivered,
not inferred from command scope or side membership. Reports carry frozen resource
values and observation times; older reports cannot replace newer observations.
Remote changes remain unknown during a broken return link. Reports and commands
compete with ordinary C2 traffic on the existing fragmented transport.

Fuel is mass in kg. Phase-specific tables interpolate altitude, airspeed and gross
mass within their documented coverage; they do not extrapolate. Reserve fuel
triggers autonomous return. Airborne exhaustion loses the aircraft; surface ship
exhaustion immobilizes it. Transfer is limited by receiver capacity, donor reserve,
station and shared-pump limits, receiver intake, connection state and operating
envelope. Durations use game time, including fractional frames and shared speeds.

Facilities enforce compatible profiles, runway dimensions, operating mass, approach
waypoints, parking and service capacity. Carrier operations check wind over deck,
crosswind and sea state. Transfers check relative speed, proximity, altitude,
speed and sea-state limits. Weather controls add north/east wind components and a
0–9 sea-state field through the existing revisioned, tick-boundary intervention
path. These are constraints on the current kinematic simulation, not a full flight,
ship-motion or deck-personnel simulator.

The exercise has one abstract ammunition stock and repair-point stock; their cargo
handling uses explicitly fictional unit masses. It does not establish compatibility
for real missile loadouts, real repair processes or real-world cargo packaging.
Carrier aviation stocks, transferable supply fuel and propulsion fuel are separate
inventories. Nuclear-powered carrier movement does not consume aviation fuel.

## Sources and coverage limits

`data/logistics/aircraft-fuel.v1.json` records all eleven existing Blue aircraft
types, known fuel facts, original units, available SI conversions, source links,
configuration applicability and missing evidence. The catalog distinguishes tank
capacity, offload rates and receiver intake rates. Gallons are not silently
converted into fuel mass without a sourced density and reference conditions.

Public manufacturer and service publications establish several capacities and
KC-46 equipment rates. They do not establish the complete variant-specific
performance tables, receiver envelopes, servicing parameters and naval operating
limits required by the selected source-only policy. No real aircraft logistics
profile is enabled in this checkpoint. F-15EX data are not borrowed from F-15E;
unknown capacity/configuration and operating limits remain explicit gaps.

Wikipedia can lead to source publications; the committed numeric facts refer to
primary publications. No licensed Jane's material was provided or incorporated.
Licensed evidence can be added without redistributing restricted source documents.
Fictional exercise values must not be copied into named real-platform profiles.

## Recovery and validation

Logistics-enabled simulation snapshots use schema 4 and preserve inventories,
reservations, partial loading/transfer, queues, equipment state, parent attachments
and delivered report knowledge. Legacy schemas 1–3 retain their previous meanings;
Regional's preceding radio checkpoint is recognized during recovery. Existing games
do not acquire the new logistics rules by loading an older save. No SQL migration
or new persistence architecture is introduced.

Focused tests cover fuel/table boundaries, exact receiver-limited transfer, reserve
protection, stock conservation, duplicate delivery, partial-transfer recovery,
carrier attachment, weather constraints, cargo delivery into carrier inventories,
shared stations, fractional rearming/cargo work and report authorization. Browser
coverage exercises delivered aerial-refueling orders, receipts, completed transfer
quantities, duplicate submissions and reload on desktop and phone.

Validation results and any unrun checks are recorded in the current implementation
checkpoint. Fictional exercise success is not real-platform performance acceptance.
