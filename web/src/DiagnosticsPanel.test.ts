import { describe, expect, it } from "vitest";
import { connectionHealth, type ClientDiagnostics } from "./DiagnosticsPanel";

describe("connection diagnostics", () => {
  const empty: ClientDiagnostics = { lastReceivedAt: null, requestMs: null, mapUpdateMs: null, failed: false };
  it("does not report connected before receiving a projection", () => {
    expect(connectionHealth(empty, 100)).toBe("Waiting");
  });
  it("distinguishes a failed request from an aging connection and recovers on receipt", () => {
    expect(connectionHealth({ ...empty, lastReceivedAt: 100, failed: true }, 200)).toBe("Reconnecting");
    expect(connectionHealth({ ...empty, lastReceivedAt: 100 }, 5101)).toBe("Stale");
    expect(connectionHealth({ ...empty, lastReceivedAt: 5100 }, 5101)).toBe("Connected");
  });
});
