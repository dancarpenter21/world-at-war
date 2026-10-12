import { Cartesian3, Cartographic, ConstantPositionProperty, EllipsoidGeodesic, JulianDate } from "cesium";

type Motion = { property: ConstantPositionProperty; target: Cartesian3; seen: number; start: number; duration: number; path?: EllipsoidGeodesic; fromHeight: number; toHeight: number };
/** Interpolates received positions only; never predicts hidden movement. */
export class SmoothPositions {
  private running = false;
  private readonly motions = new Map<string, Motion>();
  constructor(private readonly now: () => number = () => performance.now()) {}
  setRunning(running: boolean) {
    this.running = running;
    if (!running) for (const motion of this.motions.values()) { motion.property.setValue(motion.target); motion.path = undefined; }
  }
  update(id: string, property: ConstantPositionProperty, target: Cartesian3) {
    const now = this.now(), old = this.motions.get(id);
    if (old && Cartesian3.equals(old.target, target)) {
      old.property = property;
      if (!old.path) property.setValue(target);
      return;
    }
    const current = old?.property.getValue(JulianDate.now());
    const from = current && Cartographic.fromCartesian(current), to = Cartographic.fromCartesian(target);
    const duration = old ? Math.max(16, Math.min(1000, now - old.seen)) : 0;
    const motion: Motion = { property, target, seen: now, start: now, duration, fromHeight: from?.height ?? 0, toHeight: to?.height ?? 0 };
    if (this.running && old && from && to && duration > 0) {
      motion.path = new EllipsoidGeodesic(from, to);
      property.setValue(current!);
    } else property.setValue(target);
    this.motions.set(id, motion);
  }
  tick() {
    if (!this.running) return false;
    const now = this.now(); let changed = false;
    for (const motion of this.motions.values()) {
      if (!motion.path) continue;
      const fraction = Math.min(1, Math.max(0, (now - motion.start) / motion.duration));
      if (fraction === 1) { motion.property.setValue(motion.target); motion.path = undefined; }
      else {
        const point = motion.path.interpolateUsingFraction(fraction);
        point.height = motion.fromHeight + (motion.toHeight - motion.fromHeight) * fraction;
        motion.property.setValue(Cartesian3.fromRadians(point.longitude, point.latitude, point.height));
      }
      changed = true;
    }
    return changed;
  }
  retain(ids: Set<string>) { for (const id of this.motions.keys()) if (!ids.has(id)) this.motions.delete(id); }
}
