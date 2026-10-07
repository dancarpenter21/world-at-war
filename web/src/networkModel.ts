import type { IftuProjection } from "./IftuPanel";
import type { Node } from "@xyflow/react";

export type NetworkNode = { id: string; name: string; domain: string; receiver_jammed: boolean; receiver_status_known?: boolean };
export type NetworkLink = {
  id: string; from_entity_id: string; to_entity_id: string; available: boolean; jammed: number; availability_known?: boolean;
  effective_bit_rate_bps?: number | null; queued_packets: number; queued_bytes: number;
};
export const MESSAGE_STATES = ["queued", "in_transit", "delivered", "acknowledged", "retrying", "dropped", "expired"] as const;
export type MessageState = typeof MESSAGE_STATES[number];
export type MessageRecord = {
  sequence: number; state: MessageState; packet_id?: number | null; started_at_ns?: number | null; terminal_at_ns?: number | null;
  delivered_at_ns?: number | null; drop_reason?: string | null; encoded_bytes?: number[];
  message: {
    id: string; profile_id: string; rendered_text: string; fields?: Record<string, unknown>;
    header: {
      origin_role_id: string; origin_entity_id: string; recipient_entity_id: string; classification: string;
      priority: number; created_tick: number; expires_tick: number;
    };
  };
};
export type NetworkProjection = { iftu?: IftuProjection; tick: number; nodes: NetworkNode[]; links: NetworkLink[]; messages: MessageRecord[] };
export type NetworkStreamFrame = { sequence: number; resync: boolean; projection: NetworkProjection };
export type LinkFilter = "all" | "available" | "unavailable" | "jammed" | "queued";
export type NetworkFilters = { query: string; domain: string; linkState: LinkFilter; focusNodeId: string | null };
export type TopologySelection = { kind: "node" | "link"; id: string } | null;
export type NetworkFlowNode = Node<{
  terminal: NetworkNode; slot: number; incoming: number; outgoing: number; queuedPackets: number;
}, "terminal">;
export const DEFAULT_NETWORK_FILTERS: NetworkFilters = { query: "", domain: "all", linkState: "all", focusNodeId: null };

/** Preserve user positions and React Flow measurements when a live snapshot arrives. */
export function reconcileNetworkNodes(previous: NetworkFlowNode[], projection: NetworkProjection): NetworkFlowNode[] {
  const previousById = new Map(previous.map((node) => [node.id, node]));
  const terminals = new Set(projection.nodes.map((node) => node.id));
  const usedSlots = new Set(previous.filter((node) => terminals.has(node.id)).map((node) => node.data.slot));
  const metrics = new Map(projection.nodes.map((node) => [node.id, { incoming: 0, outgoing: 0, queuedPackets: 0 }]));
  for (const link of projection.links) {
    const source = metrics.get(link.from_entity_id);
    const destination = metrics.get(link.to_entity_id);
    if (!source || !destination) continue;
    source.outgoing += 1;
    source.queuedPackets += link.queued_packets;
    destination.incoming += 1;
  }
  let nextSlot = 0;
  return projection.nodes.map((terminal) => {
    const existing = previousById.get(terminal.id);
    while (usedSlots.has(nextSlot)) nextSlot += 1;
    const slot = existing?.data.slot ?? nextSlot;
    usedSlots.add(slot);
    return {
      ...existing, id: terminal.id, type: "terminal",
      position: existing?.position ?? { x: (slot % 6) * 230, y: Math.floor(slot / 6) * 126 },
      data: { terminal, slot, ...metrics.get(terminal.id)! },
      ariaLabel: `${terminal.name}, ${terminal.domain} terminal`, style: { width: 190 }
    };
  });
}

export function linkMatchesFilter(link: NetworkLink, filter: LinkFilter): boolean {
  switch (filter) {
    case "available": return link.available;
    case "unavailable": return link.availability_known !== false && !link.available;
    case "jammed": return link.jammed > 0;
    case "queued": return link.queued_packets > 0;
    default: return true;
  }
}

/** Every visible edge must have two visible endpoints, including after filtering. */
export function filterNetwork(projection: NetworkProjection, filters: NetworkFilters) {
  const query = filters.query.trim().toLocaleLowerCase();
  const neighbors = new Set(filters.focusNodeId ? [filters.focusNodeId] : []);
  if (filters.focusNodeId) {
    for (const link of projection.links) {
      if (link.from_entity_id === filters.focusNodeId) neighbors.add(link.to_entity_id);
      if (link.to_entity_id === filters.focusNodeId) neighbors.add(link.from_entity_id);
    }
  }
  const nodes = projection.nodes.filter((node) =>
    (filters.domain === "all" || node.domain === filters.domain)
    && (!query || `${node.name} ${node.domain}`.toLocaleLowerCase().includes(query))
    && (!filters.focusNodeId || neighbors.has(node.id))
  );
  const visibleIds = new Set(nodes.map((node) => node.id));
  const links = projection.links.filter((link) =>
    visibleIds.has(link.from_entity_id) && visibleIds.has(link.to_entity_id)
    && linkMatchesFilter(link, filters.linkState)
    && (!filters.focusNodeId || link.from_entity_id === filters.focusNodeId || link.to_entity_id === filters.focusNodeId)
  );
  return { nodes, links, visibleIds };
}

export function filterMessages(projection: NetworkProjection, selection: TopologySelection, state: "all" | MessageState, query: string): MessageRecord[] {
  const link = selection?.kind === "link" ? projection.links.find((item) => item.id === selection.id) : undefined;
  const search = query.trim().toLocaleLowerCase();
  const names = new Map(projection.nodes.map((node) => [node.id, node.name]));
  return projection.messages.filter((record) => {
    const header = record.message.header;
    if (state !== "all" && record.state !== state) return false;
    if (selection?.kind === "node" && header.origin_entity_id !== selection.id && header.recipient_entity_id !== selection.id) return false;
    if (selection?.kind === "link" && (!link || header.origin_entity_id !== link.from_entity_id || header.recipient_entity_id !== link.to_entity_id)) return false;
    return !search || [record.message.rendered_text, record.message.profile_id, record.drop_reason, names.get(header.origin_entity_id), names.get(header.recipient_entity_id)]
      .join(" ").toLocaleLowerCase().includes(search);
  }).sort((a, b) => b.sequence - a.sequence);
}

export function formatBitRate(rate: number | null | undefined): string {
  if (rate == null || !Number.isFinite(rate)) return "Unknown";
  if (rate >= 1_000_000_000) return `${(rate / 1_000_000_000).toFixed(2)} Gbit/s`;
  if (rate >= 1_000_000) return `${(rate / 1_000_000).toFixed(2)} Mbit/s`;
  if (rate >= 1_000) return `${(rate / 1_000).toFixed(1)} kbit/s`;
  return `${rate.toFixed(0)} bit/s`;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function isCounter(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}
function isNode(value: unknown): value is NetworkNode {
  return isObject(value) && typeof value.id === "string" && typeof value.name === "string"
    && typeof value.domain === "string" && typeof value.receiver_jammed === "boolean";
}
function isLink(value: unknown): value is NetworkLink {
  return isObject(value) && typeof value.id === "string" && typeof value.from_entity_id === "string"
    && typeof value.to_entity_id === "string" && typeof value.available === "boolean"
    && typeof value.jammed === "number" && Number.isFinite(value.jammed) && value.jammed >= 0 && value.jammed <= 1
    && (value.effective_bit_rate_bps == null || isCounter(value.effective_bit_rate_bps))
    && isCounter(value.queued_packets) && isCounter(value.queued_bytes);
}
function isMessage(value: unknown): value is MessageRecord {
  if (!isObject(value) || !isCounter(value.sequence) || !MESSAGE_STATES.includes(value.state as MessageState)
    || !isObject(value.message) || !isObject(value.message.header)) return false;
  const message = value.message;
  const header = value.message.header;
  return typeof message.id === "string" && typeof message.profile_id === "string" && typeof message.rendered_text === "string"
    && (message.fields === undefined || isObject(message.fields))
    && typeof header.origin_role_id === "string" && typeof header.origin_entity_id === "string" && typeof header.recipient_entity_id === "string"
    && typeof header.classification === "string" && isCounter(header.priority) && isCounter(header.created_tick) && isCounter(header.expires_tick)
    && (value.delivered_at_ns == null || (typeof value.delivered_at_ns === "number" && Number.isFinite(value.delivered_at_ns) && value.delivered_at_ns >= 0))
    && (value.packet_id == null || isCounter(value.packet_id))
    && [value.started_at_ns, value.terminal_at_ns].every((time) => time == null || (typeof time === "number" && Number.isFinite(time) && time >= 0))
    && (value.drop_reason == null || typeof value.drop_reason === "string")
    && (value.encoded_bytes === undefined || (Array.isArray(value.encoded_bytes) && value.encoded_bytes.every((byte) => isCounter(byte) && byte <= 255)));
}

/** A bad frame must not discard the last usable topology or crash the workspace. */
export function decodeNetworkFrame(raw: string): NetworkStreamFrame | null {
  try {
    const value: unknown = JSON.parse(raw);
    if (!isObject(value) || !isCounter(value.sequence) || typeof value.resync !== "boolean" || !isObject(value.projection)) return null;
    const projection = value.projection;
    if (!isCounter(projection.tick) || !Array.isArray(projection.nodes) || !projection.nodes.every(isNode)
      || !Array.isArray(projection.links) || !projection.links.every(isLink)
      || !Array.isArray(projection.messages) || !projection.messages.every(isMessage)) return null;
    if (new Set(projection.nodes.map((node) => node.id)).size !== projection.nodes.length
      || new Set(projection.links.map((link) => link.id)).size !== projection.links.length) return null;
    return value as NetworkStreamFrame;
  } catch {
    return null;
  }
}

/** Weapon terminals and their configured links are known; live receiver conditions need telemetry. */
export function withWeaponEndpoints(projection: NetworkProjection): NetworkProjection {
  const nodes = [...projection.nodes]; const links = [...projection.links]; const ids = new Set(nodes.map(n => n.id));
  for (const flight of projection.iftu?.weapons ?? []) {
    if (!ids.has(flight.id)) {nodes.push({id: flight.id, name: flight.weapon_name, domain: "Weapon", receiver_jammed: false, receiver_status_known: false}); ids.add(flight.id);}
    if (ids.has(flight.provider_id)) links.push({id: `weapon-link:${flight.id}`, from_entity_id: flight.provider_id, to_entity_id: flight.id, available: false, availability_known: false, jammed: 0, queued_packets: 0, queued_bytes: 0});
  }
  return {...projection, nodes, links};
}
