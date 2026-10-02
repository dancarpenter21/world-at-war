import { describe, expect, it } from "vitest";
import {
  DEFAULT_NETWORK_FILTERS, decodeNetworkFrame, filterMessages, filterNetwork, formatBitRate, reconcileNetworkNodes,
  type MessageRecord, type NetworkProjection
} from "./networkModel";

function message(id: string, sequence: number, from: string, to: string, state: MessageRecord["state"] = "delivered"): MessageRecord {
  return { sequence, state, message: {
    id, profile_id: "movement.v1", rendered_text: `Move ${id}`, fields: { north_mps: 130 },
    header: { origin_role_id: "commander", origin_entity_id: from, recipient_entity_id: to, classification: "simulation-controlled", priority: 230, created_tick: 10, expires_tick: 310 }
  } };
}
function projection(): NetworkProjection {
  return { tick: 12, nodes: [
    { id: "command", name: "Joint Command", domain: "Land", receiver_jammed: false },
    { id: "one", name: "Blue One", domain: "Air", receiver_jammed: false },
    { id: "two", name: "Blue Two", domain: "Air", receiver_jammed: true },
    { id: "relay", name: "Island Relay", domain: "Sea", receiver_jammed: false }
  ], links: [
    { id: "command-one", from_entity_id: "command", to_entity_id: "one", available: true, jammed: 0, effective_bit_rate_bps: 32_000_000, queued_packets: 3, queued_bytes: 960 },
    { id: "one-two", from_entity_id: "one", to_entity_id: "two", available: false, jammed: 1, effective_bit_rate_bps: 0, queued_packets: 0, queued_bytes: 0 },
    { id: "two-one", from_entity_id: "two", to_entity_id: "one", available: true, jammed: 0.2, effective_bit_rate_bps: 200_000, queued_packets: 0, queued_bytes: 0 }
  ], messages: [message("first", 1, "command", "one"), message("failed", 3, "one", "two", "dropped"), message("reverse", 2, "two", "one")] };
}

describe("live network reconciliation", () => {
  it("preserves dragged positions and measurements across reordered ticks while refreshing telemetry", () => {
    const snapshot = projection();
    const previous = reconcileNetworkNodes([], snapshot);
    const one = previous.find((node) => node.id === "one")!;
    one.position = { x: 813, y: 246 };
    one.measured = { width: 190, height: 82 };
    one.selected = true;
    snapshot.nodes.reverse();
    snapshot.nodes.find((node) => node.id === "one")!.receiver_jammed = true;
    snapshot.links[1].queued_packets = 7;
    const updated = reconcileNetworkNodes(previous, snapshot).find((node) => node.id === "one")!;
    expect(updated.position).toEqual({ x: 813, y: 246 });
    expect(updated.measured).toEqual(one.measured);
    expect(updated.selected).toBe(true);
    expect(updated.data.terminal.receiver_jammed).toBe(true);
    expect(updated.data).toMatchObject({ incoming: 2, outgoing: 1, queuedPackets: 7 });
  });
  it("removes vanished terminals and places new terminals in free slots", () => {
    const snapshot = projection();
    const previous = reconcileNetworkNodes([], snapshot);
    snapshot.nodes = [snapshot.nodes[2], snapshot.nodes[1], { id: "new", name: "New terminal", domain: "Cyber", receiver_jammed: false }];
    const updated = reconcileNetworkNodes(previous, snapshot);
    expect(updated.map((node) => node.id)).toEqual(["two", "one", "new"]);
    expect(updated[0].position).toEqual(previous[2].position);
    expect(updated[1].position).toEqual(previous[1].position);
    expect(new Set(updated.map((node) => node.data.slot)).size).toBe(3);
    expect(updated[2].position).toEqual({ x: 0, y: 0 });
  });
});

describe("role-visible network filters", () => {
  it("combines domain and case-insensitive name search without dangling or unknown endpoints", () => {
    const snapshot = projection();
    snapshot.links.push({ ...snapshot.links[0], id: "hidden", to_entity_id: "hidden-terminal" });
    const result = filterNetwork(snapshot, { ...DEFAULT_NETWORK_FILTERS, domain: "Air", query: "  BLUE  " });
    expect(result.nodes.map((node) => node.id)).toEqual(["one", "two"]);
    expect(result.links.map((link) => link.id)).toEqual(["one-two", "two-one"]);
    expect(filterNetwork(snapshot, DEFAULT_NETWORK_FILTERS).links).toHaveLength(3);
  });
  it("distinguishes unavailable, partially jammed, and queued directions", () => {
    const snapshot = projection();
    const filtered = (linkState: "unavailable" | "jammed" | "queued") => filterNetwork(snapshot, { ...DEFAULT_NETWORK_FILTERS, linkState }).links.map((link) => link.id);
    expect(filtered("unavailable")).toEqual(["one-two"]);
    expect(filtered("jammed")).toEqual(["one-two", "two-one"]);
    expect(filtered("queued")).toEqual(["command-one"]);
  });
  it("focuses a terminal's incident links without including connections between its neighbors", () => {
    const snapshot = projection();
    snapshot.links.push({ ...snapshot.links[0], id: "command-two", to_entity_id: "two" });
    const result = filterNetwork(snapshot, { ...DEFAULT_NETWORK_FILTERS, focusNodeId: "one" });
    expect(result.nodes.map((node) => node.id)).toEqual(["command", "one", "two"]);
    expect(result.links.map((link) => link.id)).toEqual(["command-one", "one-two", "two-one"]);
  });
  it("retains isolated terminals and reports empty search results", () => {
    const snapshot = projection();
    expect(filterNetwork(snapshot, { ...DEFAULT_NETWORK_FILTERS, domain: "Sea" })).toMatchObject({ nodes: [snapshot.nodes[3]], links: [] });
    expect(filterNetwork(snapshot, { ...DEFAULT_NETWORK_FILTERS, query: "missing" }).nodes).toEqual([]);
  });
});

describe("authorized message inspection", () => {
  it("scopes link messages to the exact direction and terminal messages to both directions", () => {
    const snapshot = projection();
    expect(filterMessages(snapshot, { kind: "link", id: "one-two" }, "all", "").map((record) => record.message.id)).toEqual(["failed"]);
    expect(filterMessages(snapshot, { kind: "node", id: "one" }, "all", "").map((record) => record.message.id)).toEqual(["failed", "reverse", "first"]);
    expect(filterMessages(snapshot, { kind: "link", id: "missing" }, "all", "")).toEqual([]);
  });
  it("combines state, content, endpoint-name, and failure searches without mutating history", () => {
    const snapshot = projection();
    snapshot.messages[1].drop_reason = "ReceiverInterference";
    expect(filterMessages(snapshot, null, "dropped", " receiverinterference ").map((record) => record.message.id)).toEqual(["failed"]);
    expect(filterMessages(snapshot, null, "delivered", "joint command").map((record) => record.message.id)).toEqual(["first"]);
    expect(filterMessages(snapshot, null, "delivered", "Move reverse").map((record) => record.message.id)).toEqual(["reverse"]);
    expect(snapshot.messages.map((record) => record.sequence)).toEqual([1, 3, 2]);
  });
});

describe("stream frames and telemetry", () => {
  it("accepts complete snapshots including nullable server telemetry", () => {
    const snapshot = projection();
    snapshot.links[0].effective_bit_rate_bps = null;
    snapshot.messages[0].delivered_at_ns = null;
    snapshot.messages[0].drop_reason = null;
    snapshot.messages[0].packet_id = 0;
    snapshot.messages[0].started_at_ns = 10_000_000_000;
    snapshot.messages[0].terminal_at_ns = 11_000_000_000;
    expect(decodeNetworkFrame(JSON.stringify({ sequence: 12, resync: true, projection: snapshot }))?.projection).toEqual(snapshot);
  });
  it("rejects malformed JSON, corrupt records, duplicate graph IDs, and invalid rates", () => {
    const raw = (snapshot: NetworkProjection) => JSON.stringify({ sequence: 12, resync: false, projection: snapshot });
    expect(decodeNetworkFrame("not json")).toBeNull();
    expect(decodeNetworkFrame('{"sequence":12,"projection":null}')).toBeNull();
    const corruptMessage = projection();
    (corruptMessage.messages[0].message as unknown as { header: null }).header = null;
    expect(decodeNetworkFrame(raw(corruptMessage))).toBeNull();
    const duplicate = projection();
    duplicate.nodes.push(duplicate.nodes[0]);
    expect(decodeNetworkFrame(raw(duplicate))).toBeNull();
    const invalidRate = projection();
    invalidRate.links[0].effective_bit_rate_bps = -1;
    expect(decodeNetworkFrame(raw(invalidRate))).toBeNull();
    for (const field of ["packet_id", "started_at_ns", "terminal_at_ns"] as const) {
      const invalidTime = projection();
      invalidTime.messages[0][field] = -1;
      expect(decodeNetworkFrame(raw(invalidTime))).toBeNull();
    }
  });
  it("shows zero rate accurately and formats radio and backbone rates in suitable units", () => {
    expect(formatBitRate(0)).toBe("0 bit/s");
    expect(formatBitRate(null)).toBe("Unknown");
    expect(formatBitRate(250_000)).toBe("250.0 kbit/s");
    expect(formatBitRate(32_000_000)).toBe("32.00 Mbit/s");
    expect(formatBitRate(1_000_000_000)).toBe("1.00 Gbit/s");
  });
});
