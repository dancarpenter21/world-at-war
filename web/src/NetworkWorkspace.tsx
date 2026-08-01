import { useEffect, useMemo, useState } from "react";
import { Background, Controls, MarkerType, MiniMap, ReactFlow, type Edge, type Node } from "@xyflow/react";
import "@xyflow/react/dist/style.css";

type NetworkNode = { id: string; name: string; domain: string; receiver_jammed: boolean };
type NetworkLink = {
  id: string; from_entity_id: string; to_entity_id: string; available: boolean; jammed: number;
  effective_bit_rate_bps?: number; queued_packets: number; queued_bytes: number;
};
type MessageRecord = {
  state: string; delivered_at_ns?: number; drop_reason?: string;
  message: { id: string; profile_id: string; rendered_text: string; header: { origin_entity_id: string; recipient_entity_id: string; priority: number; created_tick: number; expires_tick: number } };
};
type NetworkProjection = { tick: number; nodes: NetworkNode[]; links: NetworkLink[]; messages: MessageRecord[] };
type NetworkStreamFrame = { sequence: number; resync: boolean; projection: NetworkProjection };

function websocketUrl(apiBase: string, path: string): URL {
  const base = apiBase ? new URL(apiBase, window.location.href) : new URL(window.location.href);
  base.protocol = base.protocol === "https:" ? "wss:" : "ws:";
  base.pathname = path;
  base.search = "";
  return base;
}

export function NetworkWorkspace({ apiBase, gameId, playerId, roleId, onClose }: {
  apiBase: string; gameId: string; playerId: string; roleId: string; onClose: () => void;
}) {
  const [projection, setProjection] = useState<NetworkProjection | null>(null);
  const [selectedLink, setSelectedLink] = useState<string | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    let socket: WebSocket | undefined;
    let reconnectTimer: number | undefined;
    let lastSequence: number | undefined;
    const connect = () => {
      const url = websocketUrl(apiBase, `/v1/games/${gameId}/network/stream`);
      url.searchParams.set("player_id", playerId);
      url.searchParams.set("role_id", roleId);
      if (lastSequence !== undefined) url.searchParams.set("after_sequence", String(lastSequence));
      socket = new WebSocket(url);
      socket.onmessage = (event) => {
        const frame = JSON.parse(String(event.data)) as NetworkStreamFrame;
        lastSequence = frame.sequence;
        if (active) { setProjection(frame.projection); setError(""); }
      };
      socket.onerror = () => { if (active) setError("Network stream unavailable; reconnecting…"); };
      socket.onclose = () => { if (active) reconnectTimer = window.setTimeout(connect, 1000); };
    };
    connect();
    return () => {
      active = false;
      if (reconnectTimer !== undefined) window.clearTimeout(reconnectTimer);
      socket?.close();
    };
  }, [apiBase, gameId, playerId, roleId]);

  const nodes = useMemo<Node[]>(() => (projection?.nodes ?? []).map((node, index) => ({
    id: node.id,
    position: { x: (index % 8) * 190, y: Math.floor(index / 8) * 105 },
    data: { label: <div className={`network-node-card ${node.receiver_jammed ? "jammed" : ""}`}><small>{node.domain}</small><strong>{node.name}</strong></div> },
    style: { padding: 0, border: 0, background: "transparent", width: 170 }
  })), [projection]);
  const edges = useMemo<Edge[]>(() => (projection?.links ?? []).map((link) => ({
    id: link.id, source: link.from_entity_id, target: link.to_entity_id,
    animated: link.queued_packets > 0,
    markerEnd: { type: MarkerType.ArrowClosed, color: link.available ? "#59c995" : "#e36e66" },
    style: { stroke: link.available ? link.queued_packets ? "#e0b85f" : "#4a9f81" : "#d45e59", strokeWidth: link.queued_packets ? 2.4 : 1, opacity: .58 },
    data: link
  })), [projection]);
  const link = projection?.links.find((item) => item.id === selectedLink);
  const names = useMemo(() => new Map((projection?.nodes ?? []).map((node) => [node.id, node.name])), [projection]);

  return <section className="network-workspace">
    <header className="network-header"><div><strong>C2 NETWORK</strong><small>{projection ? `TICK ${projection.tick} · ${projection.nodes.length} NODES · ${projection.links.length} DIRECTIONAL LINKS` : "Loading topology"}</small></div><button className="secondary" onClick={onClose}>Back to map</button></header>
    <div className="network-body">
      <div className="network-canvas">{error ? <p className="network-error">{error}</p> : <ReactFlow nodes={nodes} edges={edges} fitView minZoom={0.08} onEdgeClick={(_, edge) => setSelectedLink(edge.id)}><Background /><Controls /><MiniMap pannable zoomable /></ReactFlow>}</div>
      <aside className="network-inspector">
        <h2>Link telemetry</h2>
        {link ? <><strong>{names.get(link.from_entity_id)} → {names.get(link.to_entity_id)}</strong><dl>
          <div><dt>Status</dt><dd>{link.available ? "Available" : "Unavailable"}</dd></div>
          <div><dt>Effective rate</dt><dd>{link.effective_bit_rate_bps ? `${(link.effective_bit_rate_bps / 1_000_000).toFixed(2)} Mbit/s` : "—"}</dd></div>
          <div><dt>Queue</dt><dd>{link.queued_packets} packets / {link.queued_bytes.toLocaleString()} bytes</dd></div>
          <div><dt>Interference</dt><dd>{Math.round(link.jammed * 100)}%</dd></div>
        </dl></> : <p className="muted">Select a directional edge.</p>}
        <h2>Authorized message history</h2>
        <div className="network-message-list">{projection?.messages.length ? projection.messages.slice().reverse().map((record) => <article key={record.message.id}><small>{record.message.profile_id} · {record.state}</small><p>{record.message.rendered_text}</p>{record.drop_reason && <span>{record.drop_reason}</span>}</article>) : <p className="muted">No messages visible to this role.</p>}</div>
      </aside>
    </div>
  </section>;
}
