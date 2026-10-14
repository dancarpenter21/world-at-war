import { describe, expect, it } from "vitest";
import { parseMapFrameRate, parseMapResolutionScale } from "./mapFrameRate";

describe("map rendering frame budget", () => {
  it("keeps the production default at sixty frames per second", () => {
    expect(parseMapFrameRate(undefined)).toBe(60);
    expect(parseMapFrameRate("")).toBe(60);
  });
  it("accepts a bounded render budget for software-rendered multiplayer clients", () => {
    expect(parseMapFrameRate("15")).toBe(15);
    expect(parseMapFrameRate("60")).toBe(60);
    expect(parseMapFrameRate("1")).toBe(1);
  });
  it("rejects invalid, fractional and excessive rates without disabling rendering", () => {
    for (const value of ["0", "-1", "61", "15.5", "NaN", "Infinity", "invalid"]) {
      expect(parseMapFrameRate(value)).toBe(60);
    }
  });
});

describe("map resolution budget", () => {
  it("preserves native resolution by default and bounds overrides", () => {
    expect(parseMapResolutionScale(undefined)).toBe(1);
    expect(parseMapResolutionScale("0.5")).toBe(0.5);
    for (const value of ["", "0", "0.1", "2", "invalid"]) expect(parseMapResolutionScale(value)).toBe(1);
  });
});
