import { describe, expect, it } from "vitest";
import { movementDescription, movementVector } from "./movement";

describe("movement courses", () => {
  it("uses clockwise bearings from north and exact stopped vectors", () => {
    expect(movementVector(0, 130)).toEqual({ north_mps: 130, east_mps: 0 });
    expect(movementVector(90, 80)).toEqual({ north_mps: 0, east_mps: 80 });
    expect(movementVector(180, 80)).toEqual({ north_mps: -80, east_mps: 0 });
    expect(movementVector(270, 80)).toEqual({ north_mps: 0, east_mps: -80 });
    expect(movementVector(360, 130)).toEqual(movementVector(0, 130));
    expect(movementVector(123, 0)).toEqual({ north_mps: 0, east_mps: 0 });
  });
  it("keeps the maximum allowed speed within the server bound at every whole-degree course", () => {
    for (let course = 0; course <= 360; course++) {
      const vector = movementVector(course, 1_000);
      expect(Math.hypot(vector.north_mps, vector.east_mps)).toBeLessThanOrEqual(1_000);
      expect(Math.hypot(vector.north_mps, vector.east_mps)).toBeCloseTo(1_000, 5);
    }
  });
  it("rejects missing, nonfinite, or out-of-range movement values", () => {
    for (const [course, speed] of [[-1, 10], [361, 10], [0, -1], [0, 1001], [NaN, 10], [0, Infinity]]) {
      expect(() => movementVector(course, speed)).toThrow(RangeError);
    }
  });
  it("describes westbound, northbound, and stopped orders", () => {
    expect(movementDescription({ north_mps: 0, east_mps: -80 })).toBe("270° at 80 m/s");
    expect(movementDescription({ north_mps: 130, east_mps: 0 })).toBe("0° at 130 m/s");
    expect(movementDescription({ north_mps: 0, east_mps: 0 })).toBe("Stopped");
  });
});