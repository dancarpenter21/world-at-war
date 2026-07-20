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

export type Side = "Blue" | "Red";
export type Position = { latitude_deg: number; longitude_deg: number; altitude_m: number };
export type Unit = { id: string; name: string; domain: string; position: Position; sidc: string; receiver_jammed: boolean };
export type Track = { track_id: string; target_side: Side; position: Position; identity_confidence: number; observed_tick: number; received_tick: number; observed_sidc: string };
export type JammingRegion = { id: string; name: string; center: Position; radius_m: number; band: { lower_hz: number; upper_hz: number }; jammed: number };
export type CommunicationLink = { id: string; from_entity_id: string; to_entity_id: string; available: boolean; jammed: number; effective_bit_rate_bps?: number };
export type Projection = { tick: number; own_units: Unit[]; tracks: Track[]; jamming_regions: JammingRegion[]; communication_links: CommunicationLink[] };

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
          record.position.setValue(position);
          this.updateName(record, unit.name);
          this.updateSymbol(record, unit.sidc, 36);
          record.color?.setValue(unit.receiver_jammed ? Color.ORANGE : Color.WHITE);
        }
      }

      for (const track of projection.tracks) {
        visibleIds.add(track.track_id);
        const position = Cartesian3.fromDegrees(track.position.longitude_deg, track.position.latitude_deg, track.position.altitude_m);
        const name = `Uncertain ${track.target_side} track`;
        let record = this.recordFor(track.track_id, "track");
        if (!record) {
          const positionProperty = new ConstantPositionProperty(position);
          const imageProperty = new ConstantProperty(this.renderSymbol(track.observed_sidc, 34));
          const entity = this.entities.add({
            id: track.track_id,
            name,
            position: positionProperty,
            billboard: { image: imageProperty, width: 42, height: 42 },
            ellipse: { semiMajorAxis: 12_000, semiMinorAxis: 8_000, material: Color.RED.withAlpha(0.16), outline: true, outlineColor: Color.RED }
          });
          record = { entity, kind: "track", sidc: track.observed_sidc, name, position: positionProperty, image: imageProperty };
          this.records.set(track.track_id, record);
        } else {
          record.position.setValue(position);
          this.updateName(record, name);
          this.updateSymbol(record, track.observed_sidc, 34);
        }
      }

      for (const [id, record] of this.records) {
        if (visibleIds.has(id)) continue;
        this.entities.remove(record.entity);
        this.records.delete(id);
      }
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
