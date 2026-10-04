import { useState } from "react";
import { apiRequest } from "./apiClient";
import type { Role } from "./AuthorityWorkspace";
import type { Position, Projection } from "./globeEntities";

export type IftuMessage = { id: number; at_ns: number; from: string; to: string; kind: string; state: string; detail: string; weapon_id?: string; fields?: unknown };
export type WeaponFlight = { id: string; launcher_id: string; weapon_name: string; provider_id: string; position: Position; observed_at_ns: number; phase: string; update_confirmation: string; retarget_capable: boolean; handoff_capable: boolean; compatible_providers: string[] };
export type IftuProjection = { weapons: WeaponFlight[]; messages: IftuMessage[] };
type Command = { action: "update" | "retarget"; track_id: string } | { action: "assign_provider"; provider_id: string } | { action: "subscribe"; source_id: string; provider_id: string };
export function IftuHistory({ messages }: { messages: IftuMessage[] }) {
  const [query, setQuery] = useState("");
  const filtered = messages.filter(m => `${m.kind} ${m.state} ${m.detail} ${m.from} ${m.to}`.toLowerCase().includes(query.toLowerCase()));
  return <section aria-label="Weapon update messages"><h3>Weapon update messages</h3><label>Search weapon messages<input type="search" value={query} onChange={e => setQuery(e.target.value)} /></label>
    <p className="muted">Observations, provider commands, and weapon updates are separate deliveries. Receipt is shown only at the receiving terminal.</p>
    {filtered.slice(-50).reverse().map((m, i) => <details key={`${m.id}:${m.at_ns}:${i}`}><summary>{(m.at_ns / 1e9).toFixed(3)} s · {m.kind.replaceAll("_", " ")} · {m.state.replaceAll("_", " ")}</summary><p>{m.detail}</p><small>{m.from} → {m.to}</small>{m.fields != null && <pre>{JSON.stringify(m.fields, null, 2)}</pre>}</details>)}
    {!filtered.length && <p>No weapon messages visible to this terminal.</p>}
  </section>;
}
export function IftuPanel({ apiBase, gameId, playerId, role, projection, canIssueOrders }: { apiBase: string; gameId: string; playerId: string; role: Role; projection: Projection; canIssueOrders: boolean }) {
  const [selected, setSelected] = useState(""); const [trackId, setTrackId] = useState(""); const [providerId, setProviderId] = useState(""); const [sourceId, setSourceId] = useState("");
  const [busy, setBusy] = useState(false); const [message, setMessage] = useState("");
  const [retry, setRetry] = useState<{ intent_id: string; issuer_role: string; target: string; kind: { Iftu: { weapon_id: string; command: Command } }; requested_tick: number } | null>(null);
  const flights = projection.iftu?.weapons ?? []; const flight = flights.find(f => f.id === selected) ?? flights[0];
  if (!flights.length) return null;
  const track = projection.tracks.find(t => t.track_id === trackId) ?? projection.tracks[0];
  const provider = flight.compatible_providers.includes(providerId) ? providerId : flight.provider_id;
  const allowed = canIssueOrders && role.command_units.includes(flight.launcher_id) && !busy;
  async function send(command?: Command) {
    if (!allowed) return;
    const intent = command ? { intent_id: crypto.randomUUID(), issuer_role: role.id, target: flight.launcher_id, kind: { Iftu: { weapon_id: flight.id, command } }, requested_tick: projection.tick } : retry;
    if (!intent) return;
    setBusy(true); setMessage("");
    try {
      await apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intent`, { method: "POST", body: JSON.stringify({ player_id: playerId, lease_generation: role.lease_generation, intent }) });
      setRetry(null); setMessage("Command submitted. Follow its delivery in Network; weapon receipt remains unconfirmed until status returns.");
    } catch (e) { setRetry(intent); setMessage(e instanceof Error ? e.message : "Submission unconfirmed. Retry uses the same command ID."); }
    finally { setBusy(false); }
  }
  return <section className="iftu-panel" aria-label="In-flight target updates"><h2>In-flight target updates</h2>
    <label>Weapon flight<select value={flight.id} onChange={e => setSelected(e.target.value)}>{flights.map(f => <option key={f.id} value={f.id}>{f.weapon_name} · {f.id.slice(-6)}</option>)}</select></label>
    <p>{flight.phase} · {flight.update_confirmation}</p><small>Last position report: {(flight.observed_at_ns / 1e9).toFixed(1)} s</small>
    <label>Target observation<select value={track?.track_id ?? ""} onChange={e => setTrackId(e.target.value)}>{projection.tracks.map(t => <option key={t.track_id} value={t.track_id}>{t.track_id.slice(-6)} · observed {t.observed_tick}</option>)}</select></label>
    <p className="muted">The provider must know this track. Automatic updates continue for its assigned observation source.</p>
    <button className="secondary" disabled={!allowed || !track} onClick={() => track && void send({ action: "update", track_id: track.track_id })}>Send target update</button>
    <button className="secondary" disabled={!allowed || !track || !flight.retarget_capable} onClick={() => track && void send({ action: "retarget", track_id: track.track_id })}>Retarget weapon</button>
    <label>Compatible provider<select value={provider} onChange={e => setProviderId(e.target.value)}>{flight.compatible_providers.map(id => <option key={id} value={id}>{projection.own_units.find(u => u.id === id)?.name ?? id}</option>)}</select></label>
    <button className="secondary" disabled={!allowed || !flight.handoff_capable || provider === flight.provider_id} onClick={() => void send({ action: "assign_provider", provider_id: provider })}>Request provider handoff</button>
    <label>Observation source<select value={sourceId} onChange={e => setSourceId(e.target.value)}><option value="">Select a known platform</option>{projection.own_units.map(u => <option key={u.id} value={u.id}>{u.name}</option>)}</select></label>
    <button className="secondary" disabled={!allowed || !sourceId} onClick={() => void send({ action: "subscribe", source_id: sourceId, provider_id: provider })}>Request target reports</button>
    {retry && <button className="secondary" disabled={!allowed} onClick={() => void send()}>Retry original command</button>}
    <p role="status">{message}</p>
  </section>;
}
