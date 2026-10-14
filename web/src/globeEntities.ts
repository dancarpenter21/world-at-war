import type { LogisticsProjection } from "./logisticsModel";
import type { GeometryStatus } from "./networkModel";
import { SmoothPositions } from "./smoothPositions";
import type { IftuProjection } from "./IftuPanel";
import {
  Cartesian2,
  Cartesian3,
  Color,
  ColorMaterialProperty,
  ConstantPositionProperty,
  ConstantProperty,
  type Entity,
  type EntityCollection
} from "cesium";

import { latestBearings, trackTiming } from "./trackTiming";

export type Side = "Blue" | "Red";
export type Position = { latitude_deg: number; longitude_deg: number; altitude_m: number };
export type Unit = { radar_emitting?: boolean | null; weapon?: { ammunition: number; range_m: number; max_track_age_ticks: number }; hit_points?: number; velocity?: { north_mps: number; east_mps: number; climb_mps: number }; following_flight_path?: boolean; id: string; name: string; domain: string; position: Position; sidc: string; receiver_jammed: boolean; observed_tick?: number; received_tick?: number };
export type Track = { track_id: string; target_side: Side | null; position: Position; identity_confidence: number; observed_tick: number; received_tick: number; observed_sidc: string; uncertainty_m?: number; evidence?: { hypotheses?: { domain: string | null; side: Side | null; probability: number }[]; modalities: string[]; covariance: number[][]; velocity: { north_mps: number; east_mps: number }; observations: string[] }; assessed_destroyed?: boolean | null };
export type JammingRegion = { id: string; name: string; center: Position; radius_m: number; band: { lower_hz: number; upper_hz: number }; jammed: number };
export type CommunicationLink = { geometry_status?: GeometryStatus; id: string; from_entity_id: string; to_entity_id: string; available: boolean; jammed: number; effective_bit_rate_bps?: number; queued_packets?: number; queued_bytes?: number };
export type ImpactReport = { intent_id: string; observed_tick: number; launched_tick: number; resolved_tick: number; hit: boolean };
export type CombatProjection = {
  mission: { title: string; status: "active" | "succeeded" | "failed"; deadline_tick: number; finished_tick: number | null; reason: string | null } | null;
  local_shots_in_flight: number; local_impacts: ImpactReport[]; received_impacts?: { report: ImpactReport; received_tick: number }[];
};
export type Weather = { wind_north_mps?: number; wind_east_mps?: number; sea_state?: number; visibility_m: number; cloud_base_m: number; cloud_top_m: number; cloud_opacity: number; precipitation: number; thermal_contrast: number };
export type Bearing = { source: string; maximum_range_m: number; id: string; receiver_position: Position; bearing_deg: number; uncertainty_deg: number; observed_tick: number; received_tick: number; emitter_class: string };
export type SensingView = { weather_reports?: { source: string; position: Position; conditions: Weather; observed_tick: number; received_tick: number }[]; bearings: Bearing[]; radar_emitting: boolean; weather: Weather; source_modalities: Record<string, string[]> };
export type Projection = { logistics?: LogisticsProjection; frame?: number; elapsed_seconds?: number; sensing?: SensingView; iftu?: IftuProjection; combat?: CombatProjection; tick: number; own_units: Unit[]; tracks: Track[]; jamming_regions: JammingRegion[]; communication_links: CommunicationLink[] };

type SymbolImage = string | HTMLImageElement | HTMLCanvasElement;
type EntityKind = "unit" | "track";
type EntityRecord = {
  entity: Entity;
  kind: EntityKind;
  sidc: string;
  name: string;
  position: ConstantPositionProperty;
  image: ConstantProperty;
  labelText?: ConstantProperty;
  color?: ConstantProperty;
};

type RegionRecord = { entity: Entity; position: ConstantPositionProperty; radius: ConstantProperty; label: ConstantProperty };
type LinkRecord = { entity: Entity; positions: ConstantProperty; color: ConstantProperty };

export class GlobeEntityReconciler {
  private readonly smoothing = new SmoothPositions();
  setAnimationEnabled(enabled: boolean) { this.smoothing.setRunning(enabled); }
  animatePositions() { return this.smoothing.tick(); }
  private readonly weaponMarkers = new Map<string, { entity: Entity; position: ConstantPositionProperty; label: ConstantProperty }>();
  private readonly uncertainty = new Map<string, Entity>();
  private readonly bearingRegions = new Map<string, Entity>();
  private readonly records = new Map<string, EntityRecord>();
  private readonly regions = new Map<string, RegionRecord>();
  private readonly links = new Map<string, LinkRecord>();

  constructor(
    private readonly entities: EntityCollection,
    private readonly renderSymbol: (sidc: string, size: number) => SymbolImage
  ) {}

  reconcile(projection: Projection) {
    const visibleIds = new Set<string>();
    this.entities.suspendEvents();
    try {
      const uncertaintyIds = new Set(projection.tracks.filter(t => t.evidence && (t.uncertainty_m ?? 0) > 0).map(t => t.track_id));
      for (const [id, entity] of this.uncertainty) if (!uncertaintyIds.has(id)) { this.entities.remove(entity); this.uncertainty.delete(id); }
      for (const track of projection.tracks) {
        const radius = track.uncertainty_m;
        if (!track.evidence || !radius || radius <= 0 || !Number.isFinite(radius)) continue;
        let ellipse = this.uncertainty.get(track.track_id);
        if (!ellipse) { ellipse = this.entities.add({ id: `uncertainty:${track.track_id}`, ellipse: { semiMajorAxis: radius, semiMinorAxis: radius, material: Color.CYAN.withAlpha(0.12), outline: true, outlineColor: Color.CYAN.withAlpha(0.4) } }); this.uncertainty.set(track.track_id, ellipse); }
        ellipse.position = new ConstantPositionProperty(Cartesian3.fromDegrees(track.position.longitude_deg, track.position.latitude_deg, track.position.altitude_m));
        const covariance = track.evidence?.covariance;
        const nn = covariance?.[0]?.[0], ee = covariance?.[1]?.[1], ne = covariance?.[0]?.[1];
        if (nn != null && ee != null && ne != null) {
          const root = Math.hypot(nn - ee, 2 * ne);
          ellipse.ellipse!.semiMajorAxis = new ConstantProperty(2.448 * Math.sqrt(Math.max(1, (nn + ee + root) / 2)));
          ellipse.ellipse!.semiMinorAxis = new ConstantProperty(2.448 * Math.sqrt(Math.max(1, (nn + ee - root) / 2)));
          ellipse.ellipse!.rotation = new ConstantProperty(-0.5 * Math.atan2(2 * ne, nn - ee));
        } else { ellipse.ellipse!.semiMajorAxis = new ConstantProperty(radius); ellipse.ellipse!.semiMinorAxis = new ConstantProperty(radius); }
      }
      const bearings = latestBearings(projection.sensing?.bearings ?? []);
      const bearingIds = new Set(bearings.map(b => b.id));
      for (const [id, entity] of this.bearingRegions) if (!bearingIds.has(id)) { this.entities.remove(entity); this.bearingRegions.delete(id); }
      for (const bearing of bearings) {
        if (this.bearingRegions.has(bearing.id)) continue;
        const origin = bearing.receiver_position;
        const points = [origin, ...[-1, 1].map(sign => {
          const angle = (bearing.bearing_deg + sign * bearing.uncertainty_deg * 2) * Math.PI / 180;
          const latitude = origin.latitude_deg * Math.PI / 180, longitude = origin.longitude_deg * Math.PI / 180;
          const distance = bearing.maximum_range_m / 6371000;
          const destination = Math.asin(Math.sin(latitude) * Math.cos(distance) + Math.cos(latitude) * Math.sin(distance) * Math.cos(angle));
          const east = longitude + Math.atan2(Math.sin(angle) * Math.sin(distance) * Math.cos(latitude), Math.cos(distance) - Math.sin(latitude) * Math.sin(destination));
          return { latitude_deg: destination * 180 / Math.PI, longitude_deg: ((east * 180 / Math.PI + 540) % 360) - 180, altitude_m: origin.altitude_m };
        })];
        const entity = this.entities.add({ id: `bearing:${bearing.id}`, polygon: { hierarchy: Cartesian3.fromDegreesArray(points.flatMap(p => [p.longitude_deg, p.latitude_deg])), material: Color.ORANGE.withAlpha(0.12), outline: true, outlineColor: Color.ORANGE } });
        this.bearingRegions.set(bearing.id, entity);
      }

      for (const unit of projection.own_units) {
        visibleIds.add(unit.id);
        const position = Cartesian3.fromDegrees(unit.position.longitude_deg, unit.position.latitude_deg, unit.position.altitude_m);
        let record = this.recordFor(unit.id, "unit");
        if (!record) {
          const positionProperty = new ConstantPositionProperty(position);
          const imageProperty = new ConstantProperty(this.renderSymbol(unit.sidc, 36));
          const labelText = new ConstantProperty(unit.name);
          const color = new ConstantProperty(unit.receiver_jammed ? Color.ORANGE : Color.WHITE);
          const entity = this.entities.add({
            id: unit.id,
            name: unit.name,
            position: positionProperty,
            billboard: { image: imageProperty, color, width: 44, height: 44 },
            label: { text: labelText, font: "12px system-ui", fillColor: Color.WHITE, pixelOffset: new Cartesian2(0, 30) }
          });
          record = { entity, kind: "unit", sidc: unit.sidc, name: unit.name, position: positionProperty, image: imageProperty, labelText, color };
          this.records.set(unit.id, record);
        } else {
          this.updateName(record, unit.name);
          this.updateSymbol(record, unit.sidc, 36);
          record.color?.setValue(unit.receiver_jammed ? Color.ORANGE : Color.WHITE);
        }
        this.smoothing.update(unit.id, record.position, position);
        const age = projection.tick - (unit.observed_tick ?? projection.tick);
        record.labelText?.setValue(age > 10 ? `${unit.name} · ${age}s old` : unit.name);
        if (age > 10) record.color?.setValue(Color.GRAY);
      }

      for (const track of projection.tracks) {
        visibleIds.add(track.track_id);
        const position = Cartesian3.fromDegrees(track.position.longitude_deg, track.position.latitude_deg, track.position.altitude_m);
        const timing = trackTiming(track, projection.tick);
        const name = `Uncertain ${track.target_side ?? "unidentified"} track · ${timing.observationLabel.toLowerCase()}`;
        const trackColor = timing.isCurrentObservation ? Color.WHITE : Color.WHITE.withAlpha(0.65);
        let record = this.recordFor(track.track_id, "track");
        if (!record) {
          const positionProperty = new ConstantPositionProperty(position);
          const imageProperty = new ConstantProperty(this.renderSymbol(track.observed_sidc, 34));
          const color = new ConstantProperty(trackColor);
          const entity = this.entities.add({
            id: track.track_id,
            name,
            position: positionProperty,
            billboard: { image: imageProperty, color, width: 42, height: 42 },
            ellipse: { semiMajorAxis: 12_000, semiMinorAxis: 8_000, material: Color.RED.withAlpha(0.16), outline: true, outlineColor: Color.RED }
          });
          record = { entity, kind: "track", sidc: track.observed_sidc, name, position: positionProperty, image: imageProperty, color };
          this.records.set(track.track_id, record);
        } else {
          record.position.setValue(position);
          this.updateName(record, name);
          this.updateSymbol(record, track.observed_sidc, 34);
          record.color?.setValue(trackColor);
        }
        if (track.uncertainty_m !== undefined && record.entity.ellipse) {
          record.entity.ellipse.semiMajorAxis = new ConstantProperty(track.uncertainty_m);
          record.entity.ellipse.semiMinorAxis = new ConstantProperty(track.uncertainty_m);
        }
      }

      for (const [id, record] of this.records) {
        if (visibleIds.has(id)) continue;
        this.entities.remove(record.entity);
        this.records.delete(id);
      }
      const weaponIds = new Set<string>();
      for (const flight of projection.iftu?.weapons ?? []) {
        weaponIds.add(flight.id);
        const p = flight.position;
        const position = Cartesian3.fromDegrees(p.longitude_deg, p.latitude_deg, p.altitude_m);
        const text = `${flight.weapon_name} · last report ${(flight.observed_at_ns / 1e9).toFixed(1)} s`;
        let marker = this.weaponMarkers.get(flight.id);
        if (!marker) {
          const positionProperty = new ConstantPositionProperty(position);
          const label = new ConstantProperty(text);
          const entity = this.entities.add({ id: `weapon:${flight.id}`, name: flight.weapon_name, position: positionProperty,
            point: { pixelSize: 7, color: Color.ORANGE }, label: { text: label, font: "11px system-ui", pixelOffset: new Cartesian2(0, 18) } });
          marker = { entity, position: positionProperty, label }; this.weaponMarkers.set(flight.id, marker);
        } else { marker.position.setValue(position); marker.label.setValue(text); }
      }
      for (const [id, marker] of this.weaponMarkers) { if (!weaponIds.has(id)) { this.entities.remove(marker.entity); this.weaponMarkers.delete(id); } }
      this.smoothing.retain(new Set(projection.own_units.map(unit => unit.id)));
      this.reconcileRegions(projection.jamming_regions);
      this.reconcileLinks(projection);
    } finally {
      this.entities.resumeEvents();
    }
  }

  focusEntities() {
    return [
      ...Array.from(this.records.values(), (record) => record.entity),
      ...Array.from(this.regions.values(), (record) => record.entity)
    ];
  }

  private reconcileRegions(regions: JammingRegion[]) {
    const visible = new Set<string>();
    for (const region of regions) {
      visible.add(region.id);
      const position = Cartesian3.fromDegrees(region.center.longitude_deg, region.center.latitude_deg, region.center.altitude_m);
      let record = this.regions.get(region.id);
      if (!record) {
        const positionProperty = new ConstantPositionProperty(position);
        const radius = new ConstantProperty(region.radius_m);
        const label = new ConstantProperty(region.name);
        const entity = this.entities.add({
          id: `jamming-region:${region.id}`,
          name: region.name,
          position: positionProperty,
          ellipse: {
            semiMajorAxis: radius,
            semiMinorAxis: radius,
            material: Color.RED.withAlpha(0.2),
            outline: true,
            outlineColor: Color.RED
          },
          label: { text: label, font: "11px system-ui", fillColor: Color.SALMON }
        });
        record = { entity, position: positionProperty, radius, label };
        this.regions.set(region.id, record);
      } else {
        record.position.setValue(position);
        record.radius.setValue(region.radius_m);
        record.label.setValue(region.name);
        record.entity.name = region.name;
      }
    }
    for (const [id, record] of this.regions) {
      if (visible.has(id)) continue;
      this.entities.remove(record.entity);
      this.regions.delete(id);
    }
  }

  private reconcileLinks(projection: Projection) {
    const positionsByUnit = new Map(projection.own_units.map((unit) => [unit.id,
      Cartesian3.fromDegrees(unit.position.longitude_deg, unit.position.latitude_deg, unit.position.altitude_m)
    ]));
    const grouped = new Map<string, CommunicationLink[]>();
    for (const link of projection.communication_links) {
      const key = [link.from_entity_id, link.to_entity_id].sort().join(":");
      grouped.set(key, [...(grouped.get(key) ?? []), link]);
    }
    const visible = new Set<string>();
    for (const [id, directionalLinks] of grouped) {
      const first = directionalLinks[0];
      const from = positionsByUnit.get(first.from_entity_id);
      const to = positionsByUnit.get(first.to_entity_id);
      if (!from || !to) continue;
      visible.add(id);
      const available = directionalLinks.filter((link) => link.available).length;
      const color = available === directionalLinks.length ? Color.LIME : available === 0 ? Color.RED : Color.ORANGE;
      let record = this.links.get(id);
      if (!record) {
        const positions = new ConstantProperty([from, to]);
        const colorProperty = new ConstantProperty(color);
        const material = new ColorMaterialProperty(colorProperty);
        const entity = this.entities.add({
          id: `communication-link:${id}`,
          name: "Directional communication link",
          polyline: { positions, material, width: 3 }
        });
        record = { entity, positions, color: colorProperty };
        this.links.set(id, record);
      } else {
        record.positions.setValue([from, to]);
        record.color.setValue(color);
      }
    }
    for (const [id, record] of this.links) {
      if (visible.has(id)) continue;
      this.entities.remove(record.entity);
      this.links.delete(id);
    }
  }

  private recordFor(id: string, kind: EntityKind) {
    const record = this.records.get(id);
    if (!record || record.kind === kind) return record;
    this.entities.remove(record.entity);
    this.records.delete(id);
    return undefined;
  }

  private updateName(record: EntityRecord, name: string) {
    if (record.name === name) return;
    record.entity.name = name;
    record.labelText?.setValue(name);
    record.name = name;
  }

  private updateSymbol(record: EntityRecord, sidc: string, size: number) {
    if (record.sidc === sidc) return;
    record.image.setValue(this.renderSymbol(sidc, size));
    record.sidc = sidc;
  }
}
