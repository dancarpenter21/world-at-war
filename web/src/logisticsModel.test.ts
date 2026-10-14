import { describe, expect, it } from "vitest";
import { transferTimeSeconds, validCargo } from "./logisticsModel";
describe("logistics quantities", () => {
  it("expresses transfer duration in game seconds and rejects unavailable rates", () => { expect(transferTimeSeconds(500, 10)).toBe(50); expect(transferTimeSeconds(500, 0)).toBeNull(); expect(transferTimeSeconds(Infinity, 10)).toBeNull(); });
  it("requires physical finite cargo and whole ammunition counts", () => { expect(validCargo({ fuel_kg: 20, ammunition: 2, repair_points: 0 })).toBe(true); expect(validCargo({ fuel_kg: 0, ammunition: 0, repair_points: 0 })).toBe(false); expect(validCargo({ fuel_kg: NaN, ammunition: 1, repair_points: 0 })).toBe(false); expect(validCargo({ fuel_kg: 1, ammunition: 1.5, repair_points: 0 })).toBe(false); });
});
