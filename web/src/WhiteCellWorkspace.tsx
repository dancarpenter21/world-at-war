import { useEffect, useRef, useState } from "react";
import { ApiError, apiRequest } from "./apiClient";
import { usePollingResource, type PollingStatus } from "./usePollingResource";
import { ObserverWorkspace } from "./ObserverWorkspace";
import type { ObserverRole } from "./observerTypes";
import type { Role } from "./AuthorityWorkspace";
import type { Projection } from "./globeEntities";

type Note = { id: string; display_name: string; tick: number; text: string };
type Overview = { roles: Role[]; notes: Note[] };
export function WhiteCellWorkspace({ apiBase, gameId, running, lobby, role, onStart, onStatus, onAccessEnded, renderingEnabled, controlError }: {
  apiBase: string; gameId: string; running: boolean; lobby: boolean; renderingEnabled: boolean; controlError: string; role: ObserverRole;
  onStart: () => void; onStatus: (status: PollingStatus, tick: number | null) => void; onAccessEnded: () => void;
}) {
  const key = `${gameId}:${role.id}:${role.lease_generation}`;
  const auth = new URLSearchParams({ role_id: role.id, lease_generation: String(role.lease_generation) }).toString();
  const overview = usePollingResource<Overview>(key, signal => apiRequest(apiBase, `/v1/games/${gameId}/white-cell?${auth}`, { signal }), 2000);
  const [tab, setTab] = useState<"map" | "plans" | "messages" | "picture" | "notes">("map");
  const [selectedRole, setSelectedRole] = useState("");
  const [text, setText] = useState("");
  const [feedback, setFeedback] = useState("");
  const [pending, setPending] = useState(false);
  const noteId = useRef<string | null>(null);
  const ended = useRef(onAccessEnded); ended.current = onAccessEnded;
  useEffect(() => { if (overview.error instanceof ApiError && [401, 403, 404].includes(overview.error.status)) ended.current(); }, [overview.error]);
  const inspection = usePollingResource<{ picture?: Projection; planning?: unknown; messages?: unknown[]; history_limit?: string; plans?: unknown; orders?: unknown }>(
    tab === "plans" || tab === "messages" || (tab === "picture" && selectedRole) ? `${key}:${tab}:${selectedRole}` : null,
    signal => apiRequest(apiBase, `/v1/games/${gameId}/white-cell/inspect?${auth}${tab === "picture" ? `&target_role_id=${selectedRole}` : tab === "plans" ? "&section=plans" : ""}`, { signal }), 2000);
  async function saveNote() {
    setPending(true); setFeedback(""); noteId.current ??= crypto.randomUUID();
    try {
      await apiRequest(apiBase, `/v1/games/${gameId}/white-cell/notes`, { method: "POST", body: JSON.stringify({ role_id: role.id, lease_generation: role.lease_generation, note_id: noteId.current, text }) });
      setText(""); noteId.current = null; overview.refresh(); setFeedback("Note shared with White Cell.");
    } catch (cause) { setFeedback((cause as Error).message); }
    finally { setPending(false); }
  }
  return <section className="white-cell-workspace" aria-label="White Cell workspace">
    <div className="white-cell-heading"><h1>White Cell · exercise referees</h1><p>Omniscient view. Team notes and inspection are private to referees.</p>
      {lobby && <button className="command" onClick={onStart}>Start scenario</button>}
      <nav className="tabs" aria-label="White Cell inspection">{(["map", "plans", "messages", "picture", "notes"] as const).map(value => <button key={value} className={tab === value ? "active" : ""} onClick={() => setTab(value)}>{({ map: "Truth map", plans: "Plans & orders", messages: "Messages", picture: "Player picture", notes: "Team notebook" })[value]}</button>)}</nav>
      {controlError && <p role="alert">{controlError}</p>}
      {overview.error && <p role="alert">{overview.error.message}</p>}
    </div>
    <div className="white-cell-map" style={{ display: tab === "map" ? undefined : "none" }}><ObserverWorkspace renderingEnabled={renderingEnabled && tab === "map"} apiBase={apiBase} gameId={gameId} running={running && tab === "map"} role={role} onStatus={onStatus} onAccessEnded={onAccessEnded} /></div>
    {tab !== "map" && <div className="white-cell-content">
      {tab === "plans" && <><h2>All plans and orders</h2><p>Includes unpublished and undelivered plans. Viewing does not publish or deliver them.</p><DataInspector title="Exercise plans" value={inspection.data?.plans} /><DataInspector title="Orders and authority decisions" value={inspection.data?.orders} /></>}
      {tab === "messages" && <><h2>Exercise messages</h2><p>{inspection.data?.history_limit}</p><DataInspector title="Retained messages and delivery state" value={inspection.data?.messages} /></>}
      {tab === "picture" && <><h2>Player picture</h2><label>Inspect operational role<select aria-label="Inspect operational role" value={selectedRole} onChange={e => setSelectedRole(e.target.value)}><option value="">Select a role</option>{overview.data?.roles.map(r => <option key={r.id} value={r.id}>{r.side} · {r.name}</option>)}</select></label>
        <p>This is the selected role's knowledge, including stale and undelivered information limits. Their knowledge is not changed.</p>
        <DataInspector title="Role knowledge" value={inspection.data?.picture} /><DataInspector title="Role plans and reports" value={inspection.data?.planning} /></>}
      {tab === "notes" && <><h2>Private team notebook</h2><label>Referee note<textarea aria-label="Referee note" maxLength={4000} disabled={pending} value={text} onChange={e => { setText(e.target.value); noteId.current = null; }} /></label>
        <button className="command" disabled={pending || !text.trim()} onClick={() => void saveNote()}>Share note</button><p role="status">{feedback}</p>
        <ol className="referee-notes">{overview.data?.notes.map(note => <li key={note.id}><strong>{note.display_name} · tick {note.tick}</strong><p>{note.text}</p></li>)}</ol></>}
      {inspection.error && <p role="alert">{inspection.error.message}</p>}
    </div>}
  </section>;
}
function label(key: string) { return key.replaceAll("_", " ").replace(/\b\w/g, value => value.toUpperCase()); }
function DataInspector({ title, value }: { title: string; value: unknown }) {
  return <details open className="white-cell-inspector"><summary>{title}</summary>{value === undefined ? <p>Waiting for current state.</p> : <InspectionValue value={value} />}</details>;
}
function InspectionValue({ value }: { value: unknown }) {
  if (value == null) return <span>None</span>;
  if (Array.isArray(value)) return value.length ? <ol>{value.map((item, index) => <li key={index}><InspectionValue value={item} /></li>)}</ol> : <span>No entries</span>;
  if (typeof value === "object") return <dl>{Object.entries(value as Record<string, unknown>).map(([key, item]) => <div key={key}><dt>{label(key)}</dt><dd>{typeof item === "object" && item !== null ? <details><summary>{Array.isArray(item) ? `${item.length} entries` : "Inspect"}</summary><InspectionValue value={item} /></details> : <InspectionValue value={item} />}</dd></div>)}</dl>;
  return <span>{typeof value === "boolean" ? value ? "Yes" : "No" : String(value)}</span>;
}
