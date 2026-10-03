export type MovementVector = { north_mps: number; east_mps: number };

export function movementVector(course: number, speed: number): MovementVector {
  if (!Number.isFinite(course) || course < 0 || course > 360 || !Number.isFinite(speed) || speed < 0 || speed > 1_000) {
    throw new RangeError("Choose a course from 0 to 360° and a speed from 0 to 1,000 m/s.");
  }
  const radians = (course % 360) * Math.PI / 180;
  const clean = (value: number) => Math.abs(value) < 1e-9 ? 0 : Math.round(value * 1e6) / 1e6;
  const north = clean(Math.cos(radians) * speed);
  const east = clean(Math.sin(radians) * speed);
  const magnitude = Math.hypot(north, east);
  const scale = magnitude > speed ? speed / magnitude * (1 - Number.EPSILON) : 1;
  return { north_mps: north * scale, east_mps: east * scale };
}

export function movementDescription(vector: MovementVector): string {
  const speed = Math.hypot(vector.north_mps, vector.east_mps);
  if (speed < 0.001) return "Stopped";
  const course = (Math.atan2(vector.east_mps, vector.north_mps) * 180 / Math.PI + 360) % 360;
  return `${Math.round(course) % 360}° at ${Math.round(speed)} m/s`;
}