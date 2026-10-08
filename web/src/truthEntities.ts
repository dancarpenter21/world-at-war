import { Cartesian2, Cartesian3, Color, ConstantPositionProperty, EllipseGraphics, EntityCollection, LabelGraphics, PointGraphics, PolylineGraphics } from "cesium";
import type { TruthProjection } from "./observerTypes";
import type { Position } from "./globeEntities";
const point = (p: Position) => Cartesian3.fromDegrees(p.longitude_deg, p.latitude_deg, p.altitude_m);
/** Dedicated truth layer: never constructs operational tracks or friendly knowledge. */
export function updateTruthEntities(entities: EntityCollection, truth: TruthProjection, network: boolean) {
  const retained = new Set<string>();
  const positions = new Map<string, Position>();
  entities.suspendEvents();
  try {
    const marker = (id: string, name: string, pose: Position, color: Color) => {
      retained.add(id);
      const entity = entities.getOrCreateEntity(id);
      entity.name = name;
      entity.position = new ConstantPositionProperty(point(pose));
      entity.point = new PointGraphics({ pixelSize: 10, color, outlineColor: Color.BLACK, outlineWidth: 1 });
      entity.label = new LabelGraphics({ text: name, font: "13px sans-serif", fillColor: color, showBackground: true, pixelOffset: new Cartesian2(0, -20) });
    };
    for (const unit of truth.units) {
      positions.set(unit.id, unit.position);
      marker(`unit:${unit.id}`, unit.name, unit.position, unit.destroyed ? Color.GRAY : unit.side === "Red" ? Color.SALMON : Color.CYAN);
    }
    for (const weapon of truth.weapons) {
      const pose = weapon.position ?? weapon.aim_point;
      if (pose) {
        if (weapon.position) positions.set(weapon.id, pose);
        marker(`weapon:${weapon.id}`, weapon.position ? `Weapon · ${weapon.phase}` : "Pending impact aim point", pose, Color.YELLOW);
      }
    }
    for (const region of truth.jamming_regions) {
      const id = `jam:${region.id}`; retained.add(id);
      const entity = entities.getOrCreateEntity(id);
      entity.name = region.name;
      entity.position = new ConstantPositionProperty(point(region.center));
      entity.ellipse = new EllipseGraphics({ semiMajorAxis: region.radius_m, semiMinorAxis: region.radius_m, material: Color.ORANGE.withAlpha(0.18) });
    }
    if (network) for (const link of truth.communication_links) {
      const from = positions.get(link.from_entity_id), to = positions.get(link.to_entity_id);
      if (!from || !to) continue;
      const id = `link:${link.id}`; retained.add(id);
      const entity = entities.getOrCreateEntity(id);
      entity.name = link.id;
      entity.polyline = new PolylineGraphics({ positions: [point(from), point(to)], width: 2, material: link.available ? Color.LIME.withAlpha(0.6) : Color.RED.withAlpha(0.7) });
    }
    for (const entity of [...entities.values]) if (!retained.has(entity.id)) entities.remove(entity);
  } finally { entities.resumeEvents(); }
}
