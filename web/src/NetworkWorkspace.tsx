import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  Background, Controls, Handle, MarkerType, MiniMap, Position, ReactFlow, useNodesState,
  type Edge, type NodeProps, type ReactFlowInstance
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import {
  DEFAULT_NETWORK_FILTERS, MESSAGE_STATES, filterMessages, filterNetwork, formatBitRate, reconcileNetworkNodes,
  type LinkFilter, type MessageRecord, type MessageState, type NetworkFilters, type NetworkFlowNode, type TopologySelection
} from "./networkModel";
import { useNetworkStream } from "./useNetworkStream";

const TerminalNode = memo(function TerminalNode({ data, selected }: NodeProps<NetworkFlowNode>) {
  return <div className={`network-node-card ${data.terminal.receiver_jammed ? "jammed" : ""} ${selected ? "selected" : ""}`}>
    <Handle type="target" position={Position.Left} isConnectable={false} />
    <div className="network-node-meta"><small>{data.terminal.domain}</small>{data.terminal.receiver_jammed && <span>Jammed</span>}</div>
    <strong title={data.terminal.name}>{data.terminal.name}</strong>
    <small>{data.incoming} in / {data.outgoing} out{data.queuedPackets > 0 ? ` · ${data.queuedPackets} queued` : ""}</small>
    <Handle type="source" position={Position.Right} isConnectable={false} />
  </div>;
});
const nodeTypes = { terminal: TerminalNode };
const emptyProjection = { tick: 0, nodes: [], links: [], messages: [] };
const labelState = (state: string) => state.replaceAll("_", " ");
const formatDuration = (nanoseconds: number) => {
  const ms = Math.max(0, nanoseconds / 1_000_000);
  return ms < 1_000 ? `${ms.toFixed(1)} ms` : `${(ms / 1_000).toFixed(2)} s`;
};

function MessageDetails({ record, name }: { record: MessageRecord; name: (id: string) => string }) {
  const { message } = record;
  const latencyMs = record.delivered_at_ns == null ? null : Math.max(0, (record.delivered_at_ns - message.header.created_tick * 1_000_000_000) / 1_000_000);
  return <section className="network-message-details" aria-label="Message details">
    <h2>Message details</h2>
    <span className={`network-state ${record.state}`}>{labelState(record.state)}</span>
    <dl>
      <div><dt>From</dt><dd>{name(message.header.origin_entity_id)}</dd></div>
      <div><dt>To</dt><dd>{name(message.header.recipient_entity_id)}</dd></div>
      <div><dt>Profile</dt><dd>{message.profile_id}</dd></div>
      <div><dt>Classification</dt><dd>{message.header.classification}</dd></div>
      <div><dt>Priority</dt><dd>{message.header.priority}</dd></div>
      <div><dt>Created</dt><dd>Tick {message.header.created_tick}</dd></div>
      <div><dt>Expires</dt><dd>Tick {message.header.expires_tick}</dd></div>
      <div><dt>Delivery time</dt><dd>{latencyMs === null ? "Not delivered" : latencyMs < 1_000 ? `${latencyMs.toFixed(1)} ms` : `${(latencyMs / 1_000).toFixed(2)} s`}</dd></div>
      {record.packet_id != null && <div><dt>Queue wait</dt><dd>{record.started_at_ns == null
        ? ["dropped", "expired"].includes(record.state) ? "Not transmitted" : "Waiting to transmit"
        : formatDuration(record.started_at_ns - message.header.created_tick * 1_000_000_000)}</dd></div>}
      {record.started_at_ns != null && <div><dt>Network transit</dt><dd>{record.terminal_at_ns == null ? "In transit" : formatDuration(record.terminal_at_ns - record.started_at_ns)}</dd></div>}
      {record.encoded_bytes && <div><dt>Encoded size</dt><dd>{record.encoded_bytes.length.toLocaleString()} bytes</dd></div>}
    </dl>
    {record.drop_reason && <p className="network-drop-reason">{record.drop_reason}</p>}
    <h3>Authorized content</h3><p className="network-message-content">{message.rendered_text || "No rendered content."}</p>
    {message.fields && Object.keys(message.fields).length > 0 && <><h3>Structured fields</h3><pre>{JSON.stringify(message.fields, null, 2)}</pre></>}
  </section>;
}

export function NetworkWorkspace({ apiBase, gameId, playerId, roleId, initialFocusNodeId, onClose }: {
  apiBase: string; gameId: string; playerId: string; roleId: string; initialFocusNodeId?: string; onClose: () => void;
}) {
  const { projection, status, notice } = useNetworkStream(apiBase, gameId, playerId, roleId);
  const [nodes, setNodes, onNodesChange] = useNodesState<NetworkFlowNode>([]);
  const [filters, setFilters] = useState<NetworkFilters>(() => ({ ...DEFAULT_NETWORK_FILTERS, focusNodeId: initialFocusNodeId ?? null }));
  const [selection, setSelection] = useState<TopologySelection>(null);
  const [tab, setTab] = useState<"topology" | "messages">("topology");
  const [messageState, setMessageState] = useState<"all" | MessageState>("all");
  const [messageQuery, setMessageQuery] = useState("");
  const [selectedMessage, setSelectedMessage] = useState<string | null>(null);
  const [messageLimit, setMessageLimit] = useState(100);
  const flow = useRef<ReactFlowInstance<NetworkFlowNode> | null>(null);
  const layoutFrame = useRef<number | undefined>(undefined);
  const inspector = useRef<HTMLElement | null>(null);
  const snapshot = projection ?? emptyProjection;
  const visible = useMemo(() => filterNetwork(snapshot, filters), [snapshot, filters]);
  const domains = useMemo(() => [...new Set(snapshot.nodes.map((node) => node.domain))].sort(), [snapshot.nodes]);
  const names = useMemo(() => new Map(snapshot.nodes.map((node) => [node.id, node.name])), [snapshot.nodes]);
  const name = (id: string) => names.get(id) ?? "Unknown terminal";
  const selectedNode = selection?.kind === "node" ? snapshot.nodes.find((node) => node.id === selection.id) : undefined;
  const selectedLink = selection?.kind === "link" ? snapshot.links.find((link) => link.id === selection.id) : undefined;
  const messages = useMemo(() => filterMessages(snapshot, selection, messageState, messageQuery), [snapshot, selection, messageState, messageQuery]);
  const message = messages.find((record) => record.message.id === selectedMessage);
  const scopedLinks = useMemo(() => selectedNode ? visible.links.filter((link) =>
    link.from_entity_id === selectedNode.id || link.to_entity_id === selectedNode.id
  ) : [], [visible.links, selectedNode]);

  useEffect(() => {
    if (projection) setNodes((previous) => reconcileNetworkNodes(previous, projection));
  }, [projection, setNodes]);
  useEffect(() => {
    if (!projection) return;
    setSelection((current) => current && !(current.kind === "node" ? projection.nodes : projection.links).some((item) => item.id === current.id) ? null : current);
    setFilters((current) => current.focusNodeId && !projection.nodes.some((node) => node.id === current.focusNodeId) ? { ...current, focusNodeId: null } : current);
  }, [projection]);
  useEffect(() => {
    const escape = (event: KeyboardEvent) => { if (event.key === "Escape" && !event.defaultPrevented) onClose(); };
    window.addEventListener("keydown", escape);
    return () => { window.removeEventListener("keydown", escape); if (layoutFrame.current !== undefined) cancelAnimationFrame(layoutFrame.current); };
  }, [onClose]);
  useEffect(() => { setMessageLimit(100); }, [selection, messageState, messageQuery]);
  useLayoutEffect(() => { if (inspector.current) inspector.current.scrollTop = 0; }, [selection?.kind, selection?.id, tab, selectedMessage]);

  const graphNodes = useMemo(() => nodes.map((node) => ({
    ...node, hidden: !visible.visibleIds.has(node.id), selected: selection?.kind === "node" && selection.id === node.id
  })), [nodes, visible.visibleIds, selection]);
  const edges = useMemo<Edge[]>(() => visible.links.map((link) => {
    const highlighted = selection?.kind === "link" ? selection.id === link.id : selection?.kind === "node"
      ? link.from_entity_id === selection.id || link.to_entity_id === selection.id : false;
    const color = !link.available ? "#ed8076" : link.queued_packets > 0 || link.jammed > 0 ? "#e0b85f" : "#59c995";
    return {
      id: link.id, source: link.from_entity_id, target: link.to_entity_id,
      animated: link.available && link.queued_packets > 0,
      markerEnd: { type: MarkerType.ArrowClosed, color, width: 15, height: 15 },
      style: { stroke: color, strokeWidth: highlighted ? 3 : link.queued_packets > 0 ? 2 : 1, opacity: highlighted ? 1 : selection ? 0.16 : 0.5 },
      ariaLabel: `${names.get(link.from_entity_id) ?? "Terminal"} to ${names.get(link.to_entity_id) ?? "terminal"}, ${link.available ? "available" : "unavailable"}`
    };
  }), [visible.links, selection, names]);

  const fitVisible = () => void flow.current?.fitView({ nodes: visible.nodes.map((node) => ({ id: node.id })), padding: 0.2, maxZoom: 1.1, duration: 250 });
  const locate = (id: string) => {
    setSelection({ kind: "node", id });
    const node = nodes.find((item) => item.id === id);
    if (node) void flow.current?.setCenter(node.position.x + 95, node.position.y + 40, { zoom: 1, duration: 250 });
  };
  const resetLayout = () => {
    if (!projection) return;
    setNodes(reconcileNetworkNodes([], projection));
    layoutFrame.current = requestAnimationFrame(fitVisible);
  };
  const availableCount = visible.links.filter((link) => link.available).length;
  const queuedPackets = visible.links.reduce((total, link) => total + link.queued_packets, 0);
  const filterActive = filters.query !== "" || filters.domain !== "all" || filters.linkState !== "all" || filters.focusNodeId !== null;
  const atMyTerminal = filters.focusNodeId === initialFocusNodeId && filters.query === "" && filters.domain === "all" && filters.linkState === "all";
  const focusMyTerminal = () => {
    if (!initialFocusNodeId) return;
    setFilters({ ...DEFAULT_NETWORK_FILTERS, focusNodeId: initialFocusNodeId });
    setSelection(null);
    setTab("topology");
  };

  return <section className="network-workspace" aria-label="C2 network workspace">
    <header className="network-header">
      <div><strong>C2 NETWORK</strong><small>{projection ? `TICK ${projection.tick} · ROLE-VISIBLE TOPOLOGY` : "Waiting for topology"}</small></div>
      <div className={`network-stream-status ${status}`} role="status"><span />{status === "live" ? "Live" : status === "reconnecting" ? "Reconnecting" : "Connecting"}</div>
      <button className="secondary" onClick={onClose}>Back to map</button>
    </header>
    <div className="network-toolbar">
      <div className="network-filters">
        <label className="network-search">Search terminals<input type="search" placeholder="Name or domain" value={filters.query} onChange={(event) => setFilters({ ...filters, query: event.target.value })} /></label>
        <label>Domain<select value={filters.domain} onChange={(event) => setFilters({ ...filters, domain: event.target.value })}><option value="all">All domains</option>{domains.map((domain) => <option key={domain}>{domain}</option>)}</select></label>
        <label>Link status<select value={filters.linkState} onChange={(event) => setFilters({ ...filters, linkState: event.target.value as LinkFilter })}><option value="all">All links</option><option value="available">Available</option><option value="unavailable">Unavailable</option><option value="jammed">Jammed</option><option value="queued">Queued traffic</option></select></label>
        <button className="secondary" disabled={!projection || visible.nodes.length === 0} onClick={fitVisible}>Fit view</button>
        <button className="secondary" disabled={!projection} onClick={resetLayout}>Reset layout</button>
        {initialFocusNodeId && names.has(initialFocusNodeId) && <button className="secondary" disabled={atMyTerminal} onClick={focusMyTerminal}>My terminal</button>}
        {filterActive && <button className="text-command" onClick={() => setFilters(DEFAULT_NETWORK_FILTERS)}>{filters.focusNodeId ? "All connections" : "Clear filters"}</button>}
      </div>
      <div className="network-summary" aria-label="Visible network summary">
        <span><strong>{visible.nodes.length}</strong> / {snapshot.nodes.length} terminals</span><span><strong>{visible.links.length}</strong> directional links</span>
        <span className="healthy"><strong>{availableCount}</strong> available</span><span className="degraded"><strong>{visible.links.length - availableCount}</strong> unavailable</span>
        <span><strong>{queuedPackets.toLocaleString()}</strong> queued packets</span>
        {filters.focusNodeId && <span className="network-focus">Connections of {name(filters.focusNodeId)}</span>}
      </div>
      {notice && <p className="network-notice" role="alert">{notice}</p>}
    </div>
    <div className="network-body">
      <div className="network-canvas">
        <ReactFlow<NetworkFlowNode> nodes={graphNodes} edges={edges} nodeTypes={nodeTypes} onNodesChange={onNodesChange}
          onInit={(instance) => { flow.current = instance; }} fitView minZoom={0.08} maxZoom={1.5} nodesConnectable={false} deleteKeyCode={null}
          onNodeClick={(_, node) => setSelection({ kind: "node", id: node.id })}
          onEdgeClick={(_, edge) => setSelection({ kind: "link", id: edge.id })} onPaneClick={() => setSelection(null)}>
          <Background gap={24} color="#243943" /><Controls showInteractive={false} /><MiniMap<NetworkFlowNode> pannable zoomable nodeColor={(node) => node.data.terminal.receiver_jammed ? "#d29a5c" : "#4a927b"} maskColor="#071116bb" />
        </ReactFlow>
        {!projection ? <div className="network-empty"><strong>Connecting to the network</strong><p>Your role's topology will appear here.</p></div>
          : visible.nodes.length === 0 ? <div className="network-empty"><strong>{snapshot.nodes.length ? "No terminals match your filters" : "No terminals visible to this role"}</strong>{filterActive && <button className="secondary" onClick={() => setFilters(DEFAULT_NETWORK_FILTERS)}>Reset filters</button>}</div>
          : visible.links.length === 0 && filterActive ? <div className="network-empty network-empty-links"><p>No links match your filters. Terminals remain visible.</p></div> : null}
        <div className="network-legend" aria-label="Link color legend"><span className="healthy">Available</span><span className="queued">Queued / interference</span><span className="degraded">Unavailable</span></div>
      </div>
      <aside ref={inspector} className="network-inspector">
        <div className="network-inspector-tabs" role="tablist" aria-label="Network inspector">
          <button role="tab" id="network-topology-tab" aria-controls="network-topology-panel" aria-selected={tab === "topology"} onClick={() => setTab("topology")}>Topology</button>
          <button role="tab" id="network-messages-tab" aria-controls="network-messages-panel" aria-selected={tab === "messages"} onClick={() => setTab("messages")}>Messages ({snapshot.messages.length})</button>
        </div>
        {selection && <button className="text-command network-clear-selection" onClick={() => setSelection(null)}>Clear selection · show all messages</button>}
        {tab === "topology" ? <div role="tabpanel" id="network-topology-panel" aria-labelledby="network-topology-tab">
          {selectedNode ? <section aria-label="Terminal details">
            <h2>Terminal details</h2><h3>{selectedNode.name}</h3><span className={`network-state ${selectedNode.receiver_jammed ? "dropped" : "delivered"}`}>{selectedNode.receiver_jammed ? "Receiver jammed" : "Receiver clear"}</span>
            <dl><div><dt>Domain</dt><dd>{selectedNode.domain}</dd></div><div><dt>Visible inbound</dt><dd>{scopedLinks.filter((link) => link.to_entity_id === selectedNode.id).length}</dd></div><div><dt>Visible outbound</dt><dd>{scopedLinks.filter((link) => link.from_entity_id === selectedNode.id).length}</dd></div><div><dt>Visible queue</dt><dd>{scopedLinks.filter((link) => link.from_entity_id === selectedNode.id).reduce((total, link) => total + link.queued_packets, 0)} packets</dd></div></dl>
            <div className="network-inspector-actions"><button className="secondary" onClick={() => setFilters({ ...DEFAULT_NETWORK_FILTERS, focusNodeId: selectedNode.id })}>Focus connections</button><button className="secondary" onClick={() => setTab("messages")}>View messages</button></div>
            <h2>Directional connections</h2><div className="network-link-list">{scopedLinks.length ? scopedLinks.map((link) => <button key={link.id} onClick={() => setSelection({ kind: "link", id: link.id })}><span>{name(link.from_entity_id)} → {name(link.to_entity_id)}</span><small className={link.available ? "healthy" : "degraded"}>{link.available ? formatBitRate(link.effective_bit_rate_bps) : "Unavailable"}</small></button>) : <p className="muted">No connections in the current view.</p>}</div>
          </section> : selectedLink ? <section aria-label="Link details">
            <h2>Link telemetry</h2><h3>{name(selectedLink.from_entity_id)} → {name(selectedLink.to_entity_id)}</h3>
            <span className={`network-state ${selectedLink.available ? "delivered" : "dropped"}`}>{selectedLink.available ? "Available" : "Unavailable"}</span>
            <dl><div><dt>Effective rate</dt><dd>{formatBitRate(selectedLink.effective_bit_rate_bps)}</dd></div><div><dt>Queue</dt><dd>{selectedLink.queued_packets} packets / {selectedLink.queued_bytes.toLocaleString()} bytes</dd></div><div><dt>Interference</dt><dd>{Math.round(selectedLink.jammed * 100)}%</dd></div></dl>
            <button className="secondary" onClick={() => setTab("messages")}>View messages on this link</button>
          </section> : <div className="network-inspector-hint"><h2>Inspect the network</h2><p>Select a terminal or directional link to inspect its status, queues, and authorized messages.</p></div>}
          <h2>Visible terminals ({visible.nodes.length})</h2><div className="network-terminal-list">{visible.nodes.map((node) => <button key={node.id} aria-label={`Inspect ${node.name}`} className={selection?.kind === "node" && selection.id === node.id ? "selected" : ""} onClick={() => locate(node.id)}><span>{node.name}</span><small>{node.domain}{node.receiver_jammed ? " · jammed" : ""}</small></button>)}</div>
        </div> : <div role="tabpanel" id="network-messages-panel" aria-labelledby="network-messages-tab">
          <h2>Authorized message history</h2><p className="muted">{selectedNode ? `To or from ${selectedNode.name}` : selectedLink ? `${name(selectedLink.from_entity_id)} → ${name(selectedLink.to_entity_id)}` : "Messages your role is allowed to read."}</p>
          <div className="network-message-filters"><label>Search messages<input type="search" value={messageQuery} placeholder="Content, sender, or profile" onChange={(event) => setMessageQuery(event.target.value)} /></label><label>Message state<select value={messageState} onChange={(event) => setMessageState(event.target.value as "all" | MessageState)}><option value="all">All states</option>{MESSAGE_STATES.map((state) => <option key={state} value={state}>{labelState(state)}</option>)}</select></label></div>
          {message && <MessageDetails record={message} name={name} />}
          <div className="network-message-list">{messages.length ? messages.slice(0, messageLimit).map((record) => <button key={record.message.id} aria-label={`Inspect message: ${record.message.rendered_text || record.message.profile_id}`} className={selectedMessage === record.message.id ? "selected" : ""} onClick={() => setSelectedMessage(record.message.id)}><div><small>{record.message.profile_id}</small><span className={`network-state ${record.state}`}>{labelState(record.state)}</span></div><p>{record.message.rendered_text || "No rendered content"}</p><small>{name(record.message.header.origin_entity_id)} → {name(record.message.header.recipient_entity_id)} · tick {record.message.header.created_tick}</small></button>) : <p className="muted">{snapshot.messages.length ? "No messages match the current selection and filters." : "No messages visible to this role yet."}</p>}</div>
          {messages.length > messageLimit && <button className="secondary network-load-more" onClick={() => setMessageLimit((limit) => limit + 100)}>Show more ({messages.length - messageLimit} remaining)</button>}
        </div>}
      </aside>
    </div>
  </section>;
}
