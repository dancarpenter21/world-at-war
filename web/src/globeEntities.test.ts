import { Cartesian3, EntityCollection, JulianDate } from "cesium";
import { describe, expect, it, vi } from "vitest";
import { GlobeEntityReconciler, type Projection } from "./globeEntities";

const unitSidc = "100301000011010000000000000000";
const trackSidc = "100601000011010000000000000000";

function projection(overrides: Partial<Projection> = {}): Projection {
  return {
    tick: 1,
    own_units: [{
      id: "blue-1",
      name: "Blue One",
      domain: "Air",
      position: { latitude_deg: 34, longitude_deg: -117, altitude_m: 1_000 },
      sidc: unitSidc
    }],
    tracks: [{
      track_id: "track-1",
      target_side: "Red",
      position: { latitude_deg: 35, longitude_deg: -116, altitude_m: 2_000 },
      identity_confidence: 0.75,
      observed_tick: 1,
      received_tick: 1,
      observed_sidc: trackSidc
    }],
    ...overrides
  };
}

describe("GlobeEntityReconciler", () => {
  it("preserves entities and symbols while updating positions and names", () => {
    const entities = new EntityCollection();
    const renderSymbol = vi.fn((sidc: string, size: number) => `${sidc}:${size}`);
    const reconciler = new GlobeEntityReconciler(entities, renderSymbol);
    reconciler.reconcile(projection());
    const unit = entities.getById("blue-1");
    const track = entities.getById("track-1");

    reconciler.reconcile(projection({
      tick: 2,
      own_units: [{
        id: "blue-1",
        name: "Blue One Renamed",
        domain: "Air",
        position: { latitude_deg: 36, longitude_deg: -115, altitude_m: 3_000 },
        sidc: unitSidc
      }]
    }));

    expect(entities.getById("blue-1")).toBe(unit);
    expect(entities.getById("track-1")).toBe(track);
    expect(unit?.name).toBe("Blue One Renamed");
    expect(unit?.label?.text?.getValue()).toBe("Blue One Renamed");
    expect(Cartesian3.equals(unit?.position?.getValue(JulianDate.now()), Cartesian3.fromDegrees(-115, 36, 3_000))).toBe(true);
    expect(renderSymbol).toHaveBeenCalledTimes(2);
  });

  it("changes an image only when its SIDC changes", () => {
    const entities = new EntityCollection();
    const renderSymbol = vi.fn((sidc: string, size: number) => `${sidc}:${size}`);
    const reconciler = new GlobeEntityReconciler(entities, renderSymbol);
    reconciler.reconcile(projection());
    const unit = entities.getById("blue-1");
    const changedSidc = "100301000012110000000000000000";

    reconciler.reconcile(projection({
      tick: 2,
      own_units: [{
        id: "blue-1",
        name: "Blue One",
        domain: "Air",
        position: { latitude_deg: 34, longitude_deg: -117, altitude_m: 1_000 },
        sidc: changedSidc
      }]
    }));

    expect(entities.getById("blue-1")).toBe(unit);
    expect(unit?.billboard?.image?.getValue()).toBe(`${changedSidc}:36`);
    expect(renderSymbol).toHaveBeenCalledTimes(3);
  });

  it("adds new entities and removes stale entities", () => {
    const entities = new EntityCollection();
    const reconciler = new GlobeEntityReconciler(entities, (sidc, size) => `${sidc}:${size}`);
    reconciler.reconcile(projection());

    reconciler.reconcile(projection({
      tick: 2,
      own_units: [],
      tracks: [{
        track_id: "track-2",
        target_side: "Red",
        position: { latitude_deg: 37, longitude_deg: -114, altitude_m: 4_000 },
        identity_confidence: 0.4,
        observed_tick: 2,
        received_tick: 2,
        observed_sidc: trackSidc
      }]
    }));

    expect(entities.getById("blue-1")).toBeUndefined();
    expect(entities.getById("track-1")).toBeUndefined();
    expect(entities.getById("track-2")).toBeDefined();
    expect(entities.values).toHaveLength(1);
  });
});
