import {
  Cartesian2,
  Cartesian3,
  Color,
  ConstantPositionProperty,
  ConstantProperty,
  type Entity,
  type EntityCollection
} from "cesium";

export type Side = "Blue" | "Red";
export type Position = { latitude_deg: number; longitude_deg: number; altitude_m: number };
export type Unit = { id: string; name: string; domain: string; position: Position; sidc: string };
export type Track = { track_id: string; target_side: Side; position: Position; identity_confidence: number; observed_tick: number; received_tick: number; observed_sidc: string };
export type Projection = { tick: number; own_units: Unit[]; tracks: Track[] };

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
};

export class GlobeEntityReconciler {
  private readonly records = new Map<string, EntityRecord>();

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
          const entity = this.entities.add({
            id: unit.id,
            name: unit.name,
            position: positionProperty,
            billboard: { image: imageProperty, width: 44, height: 44 },
            label: { text: labelText, font: "12px system-ui", fillColor: Color.WHITE, pixelOffset: new Cartesian2(0, 30) }
          });
          record = { entity, kind: "unit", sidc: unit.sidc, name: unit.name, position: positionProperty, image: imageProperty, labelText };
          this.records.set(unit.id, record);
        } else {
          record.position.setValue(position);
          this.updateName(record, unit.name);
          this.updateSymbol(record, unit.sidc, 36);
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
    } finally {
      this.entities.resumeEvents();
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
