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

export function denseSessionProjection(): Projection {
  const projection = sessionProjection();
  for (let index = 1; index < 64; index++) {
    projection.own_units.push({ ...projection.own_units[0], id: `blue-${index + 1}`, name: `Blue ${index + 1}`,
      position: { latitude_deg: 34 + index / 100, longitude_deg: -118, altitude_m: 8_000 } });
  }
  for (const from of projection.own_units) {
    for (const to of projection.own_units) {
      if (from.id === to.id) continue;
      const failed = from.id === "blue-one" && to.id === "blue-2";
      const queued = from.id === "blue-one" && to.id === "blue-3";
      projection.communication_links.push({ id: `${from.id}:${to.id}`, from_entity_id: from.id, to_entity_id: to.id,
        available: !failed, jammed: failed ? 1 : 0, effective_bit_rate_bps: failed ? undefined : 1_000_000,
        queued_packets: queued ? 3 : 0, queued_bytes: queued ? 300 : 0 });
    }
  }
  return projection;
}