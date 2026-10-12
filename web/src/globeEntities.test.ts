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
      sidc: unitSidc,
      receiver_jammed: false
    }],
    jamming_regions: [],
    communication_links: [],
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
        sidc: unitSidc,
        receiver_jammed: false
      }]
    }));

    expect(entities.getById("blue-1")).toBe(unit);
    expect(entities.getById("track-1")).toBe(track);
    expect(unit?.name).toBe("Blue One Renamed");
    expect(unit?.label?.text?.getValue()).toBe("Blue One Renamed");
    expect(Cartesian3.equals(unit?.position?.getValue(JulianDate.now()), Cartesian3.fromDegrees(-115, 36, 3_000))).toBe(true);
    expect(renderSymbol).toHaveBeenCalledTimes(2);
  });

  it("ages retained tracks without recreating their entities or symbols", () => {
    const entities = new EntityCollection();
    const renderSymbol = vi.fn((sidc: string, size: number) => `${sidc}:${size}`);
    const reconciler = new GlobeEntityReconciler(entities, renderSymbol);
    reconciler.reconcile(projection());
    const track = entities.getById("track-1");
    expect(track?.name).toContain("observed this tick");
    expect(track?.billboard?.color?.getValue().alpha).toBe(1);
    const position = track?.position?.getValue(JulianDate.now());
    reconciler.reconcile(projection({ tick: 26 }));
    expect(entities.getById("track-1")).toBe(track);
    expect(track?.name).toContain("observed 25s ago");
    expect(track?.billboard?.color?.getValue().alpha).toBe(0.65);
    expect(Cartesian3.equals(track?.position?.getValue(JulianDate.now()), position)).toBe(true);
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
        sidc: changedSidc,
        receiver_jammed: false
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

  it("renders jammer regions and aggregates directional link health", () => {
    const entities = new EntityCollection();
    const reconciler = new GlobeEntityReconciler(entities, (sidc, size) => `${sidc}:${size}`);
    const secondUnit = {
      id: "blue-2", name: "Blue Two", domain: "Air",
      position: { latitude_deg: 34.1, longitude_deg: -116.9, altitude_m: 1_000 },
      sidc: unitSidc, receiver_jammed: false
    };
    reconciler.reconcile(projection({
      own_units: [...projection().own_units, secondUnit],
      jamming_regions: [{
        id: "jammer", name: "Training Jammer",
        center: { latitude_deg: 34, longitude_deg: -117, altitude_m: 0 },
        radius_m: 5_000, band: { lower_hz: 300, upper_hz: 320 }, jammed: 1
      }],
      communication_links: [
        { id: "one-two", from_entity_id: "blue-1", to_entity_id: "blue-2", available: true, jammed: 0, effective_bit_rate_bps: 1_000_000 },
        { id: "two-one", from_entity_id: "blue-2", to_entity_id: "blue-1", available: false, jammed: 1 }
      ]
    }));

    expect(entities.getById("jamming-region:jammer")?.ellipse).toBeDefined();
    expect(entities.getById("communication-link:blue-1:blue-2")?.polyline).toBeDefined();
    expect(reconciler.focusEntities()).toHaveLength(4);
  });
});

it("removes uncertainty and bearing overlays when their authorized knowledge expires", () => {
  const entities = new EntityCollection(); const reconciler = new GlobeEntityReconciler(entities, () => "symbol");
  const value = projection(); value.tracks[0].uncertainty_m = 100;
  value.tracks[0].evidence = { modalities: ["radar"], covariance: [[100, 0], [0, 400]], velocity: { north_mps: 0, east_mps: 0 }, observations: ["observation"] };
  value.sensing = { radar_emitting: true, weather: { visibility_m: 100000, cloud_base_m: 0, cloud_top_m: 0, cloud_opacity: 0, precipitation: 0, thermal_contrast: 1 }, source_modalities: {}, bearings: [{ source: "receiver", maximum_range_m: 20000, id: "bearing", receiver_position: { latitude_deg: 89.9, longitude_deg: 179.9, altitude_m: 1000 }, bearing_deg: 45, uncertainty_deg: 2, observed_tick: 1, received_tick: 2, emitter_class: "fictional" }] };
  reconciler.reconcile(value); expect(entities.getById("uncertainty:track-1")).toBeDefined(); expect(entities.getById("bearing:bearing")).toBeDefined();
  reconciler.reconcile(projection({ tracks: [] })); expect(entities.getById("uncertainty:track-1")).toBeUndefined(); expect(entities.getById("bearing:bearing")).toBeUndefined();
});
