import type { MessageRecord, NetworkProjection } from "../../src/networkModel";

function message(id: string, sequence: number, from: string, to: string, text: string, state: MessageRecord["state"]): MessageRecord {
  const payload = {
    id, profile_id: "movement.v1", rendered_text: text, fields: { north_mps: 130, east_mps: 0 },
    header: { origin_role_id: "test-role", origin_entity_id: from, recipient_entity_id: to, classification: "simulation-controlled", priority: 230, created_tick: 10, expires_tick: 310 }
  };
  return { sequence, state, delivered_at_ns: state === "delivered" ? 10_120_000_000 : null, drop_reason: state === "dropped" ? "ReceiverInterference" : null,
    encoded_bytes: Array.from(new TextEncoder().encode(JSON.stringify(payload))), message: payload };
}
export function networkProjection(): NetworkProjection {
  return {
    tick: 12,
    nodes: [
      { id: "command", name: "Joint Command", domain: "Land", receiver_jammed: false },
      { id: "blue-one", name: "Blue One", domain: "Air", receiver_jammed: false },
      { id: "blue-two", name: "Blue Two", domain: "Air", receiver_jammed: true },
      { id: "relay", name: "Island Relay", domain: "Sea", receiver_jammed: false }
    ],
    links: [
      { id: "command-one", from_entity_id: "command", to_entity_id: "blue-one", available: true, jammed: 0, effective_bit_rate_bps: 32_000_000, queued_packets: 3, queued_bytes: 960 },
      { id: "one-command", from_entity_id: "blue-one", to_entity_id: "command", available: true, jammed: 0, effective_bit_rate_bps: 250_000, queued_packets: 0, queued_bytes: 0 },
      { id: "one-two", from_entity_id: "blue-one", to_entity_id: "blue-two", available: false, jammed: 1, effective_bit_rate_bps: 0, queued_packets: 0, queued_bytes: 0 },
      { id: "two-one", from_entity_id: "blue-two", to_entity_id: "blue-one", available: true, jammed: 0.2, effective_bit_rate_bps: 200_000, queued_packets: 0, queued_bytes: 0 }
    ],
    messages: [
      message("move", 1, "command", "blue-one", "Move north at 130 m/s", "delivered"),
      message("blocked", 2, "blue-one", "blue-two", "Movement order blocked by receiver jamming", "dropped"),
      message("reverse", 3, "blue-two", "blue-one", "Return to formation", "delivered")
    ]
  };
}

export function denseNetworkProjection(): NetworkProjection {
  const dense = networkProjection();
  dense.tick = 20;
  dense.nodes = Array.from({ length: 64 }, (_, index) => ({
    id: `unit-${index}`, name: `Blue unit ${index + 1}`, domain: ["Air", "Land", "Sea", "Cyber"][index % 4], receiver_jammed: index === 7
  }));
  dense.links = dense.nodes.flatMap((from, fromIndex) => dense.nodes.filter((to) => to.id !== from.id).map((to) => ({
    id: `${from.id}-${to.id}`, from_entity_id: from.id, to_entity_id: to.id,
    available: !to.receiver_jammed, jammed: to.receiver_jammed ? 1 : 0, effective_bit_rate_bps: to.receiver_jammed ? 0 : 32_000_000,
    queued_packets: fromIndex === 0 && to.id === "unit-4" ? 3 : 0, queued_bytes: fromIndex === 0 && to.id === "unit-4" ? 960 : 0
  })));
  return dense;
}
