import { describe, expect, it } from "vitest";
import { EntityCollection, JulianDate } from "cesium";
import { updateTruthEntities } from "./truthEntities";
import type { TruthProjection } from "./observerTypes";
const position = { latitude_deg: 38, longitude_deg: -76, altitude_m: 4000 };
const truth: TruthProjection = { tick: 12,
  units: [{ id: "red", name: "Hidden enemy", side: "Red", domain: "Air", position, sidc: "", receiver_jammed: false, fuel: 100, ammunition: 2, destroyed: false }],
  weapons: [{ id: "weapon", kind: "iftu", side: "Blue", position: { ...position, longitude_deg: -77 }, aim_point: null, phase: "midcourse", impact_tick: null, launcher_id: null, provider_id: null }],
  communication_links: [{ id: "temporary", from_entity_id: "red", to_entity_id: "weapon", available: false, jammed: 1, queued_packets: 2, queued_bytes: 128 }], jamming_regions: [] };
describe("truth map lifecycle", () => {
  it("draws temporary weapon endpoints and clears removed units, weapons and links", () => {
    const entities = new EntityCollection();
    updateTruthEntities(entities, truth, true);
    expect(entities.values.map(e => e.id).sort()).toEqual(["link:temporary", "unit:red", "weapon:weapon"]);
    expect(entities.getById("link:temporary")?.polyline?.positions?.getValue(JulianDate.now())).toHaveLength(2);
    updateTruthEntities(entities, { ...truth, weapons: [], units: [] }, true);
    expect(entities.values).toHaveLength(0);
  });
  it("labels pending aim points without inventing a weapon position or network endpoint", () => {
    const entities = new EntityCollection();
    updateTruthEntities(entities, { ...truth, weapons: [{ ...truth.weapons[0], position: null, aim_point: position, kind: "training_pending_impact" }] }, true);
    expect(entities.getById("weapon:weapon")?.name).toBe("Pending impact aim point");
    expect(entities.getById("link:temporary")).toBeUndefined();
    updateTruthEntities(entities, truth, false);
    expect(entities.getById("link:temporary")).toBeUndefined();
  });
});
