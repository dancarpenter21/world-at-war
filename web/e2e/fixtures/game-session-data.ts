import type { Role, AuthorityDefinition } from "../../src/AuthorityWorkspace";
import type { Game } from "../../src/GameSessionControls";
import type { Projection } from "../../src/globeEntities";

export const PLAYER_ID = "session-test-player";

export function sessionGame(): Game {
  return { id: "session-test", title: "Jammed Flight Test", status: "lobby", host_player_id: PLAYER_ID, player_roles_available: 1, space_catalog_enabled: false, operational_error: null };
}

export function sessionRole(): Role {
  return { id: "blue-pilot", name: "Blue One Pilot", side: "Blue", kind: "pilot", location_unit_id: "blue-one", command_units: ["blue-one"], held: false, ai_controlled: false, lease_generation: 0 };
}

export function sessionAuthority(): AuthorityDefinition {
  return {
    version: 1,
    roles: [{ id: "blue-pilot", name: "Blue One Pilot", side: "Blue", kind: "pilot", location_unit_id: "blue-one", claimable: true, ai_controlled: false }],
    relationships: [], policies: []
  };
}

export function sessionProjection(tick = 12): Projection {
  return {
    tick,
    own_units: [{ id: "blue-one", name: "Blue One", domain: "Air", position: { latitude_deg: 34, longitude_deg: -118, altitude_m: 8_000 }, sidc: "10031000001101000000", receiver_jammed: false }],
    tracks: [], jamming_regions: [], communication_links: []
  };
}