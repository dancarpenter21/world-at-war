# World At War

World at War is a realistic low-fidelity  war simulator featuring realistic vision, sensors, communications, unit movement, doctrine, and authorities across the air, space, cyber, sea, undersea, and land domains.

See [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) for the phased architecture, multiplayer design, simulation model, platform catalog, and acceptance criteria.

## Development

The implemented global-crisis scenario contains 64 authored entities, thirteen claimable Blue command roles, a basic Red AI, a doctrine-oriented authority graph, White House and Pentagon command nodes, and a pinned public orbital catalog. Entities and uncertain tracks use MIL-STD-2525D SIDCs rendered with milsymbol.

```sh
cargo test
cargo run --package world-at-war-server

# In another terminal
cd web
npm install
npm run dev
```

The API runs on `http://localhost:8000` and the Vite client on `http://localhost:5173`. To run the production-shaped local stack, use `docker compose up --build` and open `http://localhost:8080` (or the `.env` value of `APP_PORT`). Compose publishes only its Nginx edge proxy: it forwards frontend and Cesium assets to the internal `web` container, and `/health`, `/v1/`, and WebSocket upgrades such as `/v1/games/{game_id}/stream` to the internal Rust server. Set `COOKIE_SECURE=true` when an HTTPS proxy terminates TLS in front of this stack.

The first screen is a scenario/join modal; Cesium is not constructed until a player holds a role in a running game. Enter Space-Track credentials in the modal to fetch the current public GP catalog. Selecting **Remember credentials** stores authenticated encrypted data in a 30-day `HttpOnly`, `SameSite=Strict` cookie; the encryption key remains in the ignored server cache volume. Set `COOKIE_SECURE=true` when serving over HTTPS. Catalog snapshots are cached under ignored `data/cache/space-track/`. `SPACETRACK_USERNAME` and `SPACETRACK_PASSWORD` may instead be supplied through the server environment. For a hosted deployment, setting the optional `ADMIN_SETUP_TOKEN` restores bearer-token protection for catalog setup and makes the token field appear in the modal.

The host can open **Configure authorities** before or during play. The graph distinguishes operational command from support, advisory, and order-transmission relationships; policies define direct grants, requesters, ordered approval roles, and deterministic vacant-role decisions. Players use the same workspace as a read-only command-chain view and authority-request inbox. Communications currently pass through an always-reachable gate so the later boolean communications graph can replace it without changing authority policy data.

## Archtecture

Rust entity component system to track unit positions and do all line of sight calculations.

Cesium map web front end simple display client.

Docker and Docker Compose to run and test.

## Domains

Pulls realistic satellite tracks from Spacetrack and other space catalog sites.   
Ground units are amorphous shapes that represent the area of land that ground unit controls.  
Realistic naval ships, airplanes, weapons, communication systems.   
Pull entity information for platforms from Jane's and other open source platform.   

ICAO codes and runway locations and lengths for all airports in the world.

## Roles

When a player joins a game they get to pick what role they would like to play. They can select a unit and control that unit, or play as the President of SecDef. Whateveer role the player selects, they can only see the entities that are in field of view and to which they have communications. Each scenario has the White House situation room and Pentagon with all authorities and communication chains originating at the White House to the Pentagon. From there the game's communication system must model the links that work to bases, op centers, down to tactical units. Realistic communcation systems over SATCOM, tactical radio links, open internet, etc. 

The game's vision system relies on realistic sensor modeling. Direct line of sight, radar, satellite imagery, all comprise the vision system, but a unit that can see an enemy entity may not be able to communicate that entity's position to its commmand center (e.g. a satellite may be jammed or may be on the wrong side of the planet). Vision type is a consideration as well: a sigint sensor may not be able to tell exactly what or where an emanating platform is, it may only identify an ELNOT in a probabilstic region. A JTAC with eyes on a target needs to be able to communicate.

### Authorities

Units may only execute orders if their authority chain allows it, and that entity has the capability to execute the action. 
