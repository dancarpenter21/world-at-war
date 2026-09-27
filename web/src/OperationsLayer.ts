import { Cartesian3, Color, EntityCollection, PolygonHierarchy } from "cesium";
import type { PlanningView } from "./PlanningWorkspace";

/** Only receives the role's delivered planning projection. */
export class OperationsLayer {
  private ids: string[] = [];
  private revision = "";
  constructor(private readonly entities: EntityCollection) {}
  update(view: PlanningView) {
    const plan = view.received;
    const active = plan?.airspaces.filter(a => a.start_tick <= view.tick && view.tick < a.end_tick) ?? [];
    const key = `${plan?.id}:${plan?.revision}:${active.map(a => a.id).join()}`;
    if (key === this.revision) return;
    this.clear(); this.revision = key;
    for (const volume of active) {
      const id = `airspace-${volume.id}`; this.ids.push(id);
      this.entities.add({ id, name: `${volume.name} · ${volume.control_method}`, polygon: { hierarchy: new PolygonHierarchy(volume.polygon.map(p => Cartesian3.fromDegrees(p.longitude_deg,p.latitude_deg))), height: volume.floor_m, extrudedHeight: volume.ceiling_m, material: (volume.control_method === "positive" ? Color.ORANGE : Color.CYAN).withAlpha(0.08), outline: true, outlineColor: Color.CYAN.withAlpha(0.6) } });
    }
    const course = plan?.courses.find(c => c.id === plan.selected_course_id);
    for (const mission of course?.missions ?? []) {
      const id = `mission-route-${mission.id}`; this.ids.push(id);
      this.entities.add({ id, name: `${mission.name}: ${mission.start_tick}–${mission.end_tick}`, polyline: { positions: mission.route.map(p => Cartesian3.fromDegrees(p.longitude_deg,p.latitude_deg,p.altitude_m)), width: 2, material: Color.YELLOW.withAlpha(0.65) } });
    }
  }
  clear() { this.ids.forEach(id => this.entities.removeById(id)); this.ids = []; this.revision = ""; }
}
