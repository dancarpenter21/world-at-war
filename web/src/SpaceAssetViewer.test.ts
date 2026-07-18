import { describe, expect, it } from "vitest";
import { publicValue } from "./SpaceAssetViewer";

describe("publicValue", () => {
  it("redacts absent or unknown public satellite data", () => {
    expect(publicValue(undefined)).toBe("[REDACTED]");
    expect(publicValue("Unknown")).toBe("[REDACTED]");
    expect(publicValue("  ")).toBe("[REDACTED]");
  });

  it("retains reviewed public data", () => {
    expect(publicValue("Advanced Baseline Imager (ABI)")).toBe("Advanced Baseline Imager (ABI)");
  });
});
