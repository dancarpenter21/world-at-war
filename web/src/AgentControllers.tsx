import { useEffect, useState } from "react";
import { apiRequest } from "./apiClient";

type Control = "human" | "deterministic" | "external";
type View = {
  roles: { role_id: string; name: string; side: string; mode: Control; fallback: boolean; assignment_generation: number | null; deadline_unix_ms: number | null; observation: unknown }[];
  history: { role_id: string; tick: number; kind: string; details: unknown }[];
};
export function AgentControllers({ apiBase, gameId, onChanged }: { apiBase: string; gameId: string; onChanged: () => void }) {
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  const [credential, setCredential] = useState<{ token: string; role: string } | null>(null);
  const [timeout, setTimeoutSeconds] = useState(30);
  const path = `/v1/games/${gameId}/agents`;
  useEffect(() => {
    let active = true;
    async function refresh() {
      try { const data = await apiRequest<View>(apiBase, path); if (active) { setView(data); setError(""); } }
      catch (cause) { if (active) setError((cause as Error).message); }
    }
    void refresh(); const timer = window.setInterval(() => void refresh(), 5000);
    return () => { active = false; window.clearInterval(timer); };
  }, [apiBase, path]);
  async function assign(role: string, mode: Control) {
    setPending(true); setError(""); setCredential(null);
    try {
      const result = await apiRequest<{ token: string | null }>(apiBase, `${path}/${role}`, { method: "POST", body: JSON.stringify({ mode, timeout_ms: timeout * 1000 }) });
      if (result.token) setCredential({ token: result.token, role });
      setView(await apiRequest<View>(apiBase, path)); onChanged();
    } catch (cause) { setError((cause as Error).message); }
    finally { setPending(false); }
  }
  return <section aria-label="AI controllers">
    <h3>AI controllers</h3>
    <p>Assign a human, built-in AI, or an external agent to each role. Reassignment releases its current player. External agents use only that role’s information.</p>
    <label>Agent response timeout (seconds) <input type="number" min={1} max={300} value={timeout} onChange={e => setTimeoutSeconds(Number(e.target.value))} /></label>
    {error && <p role="alert">{error}</p>}
    {credential && <div role="status"><p>Connection token for role {credential.role}. Copy it now; it is shown once.</p><code style={{ overflowWrap: "anywhere" }}>{credential.token}</code>
      <button className="secondary" onClick={() => { void navigator.clipboard.writeText(credential.token).catch(() => setError("Copy the displayed token manually.")); }}>Copy token</button>
      <button className="secondary" onClick={() => setCredential(null)}>Hide token</button></div>}
    {!view ? <p>Loading controllers…</p> : <>
      {view.roles.map(role => <div className="team-request" key={role.role_id}>
        <span>{role.name} · {role.side} · {role.mode === "external" ? role.fallback ? "External agent · built-in fallback" : "External agent" : role.mode === "deterministic" ? "Built-in AI" : "Human"}</span>
        <select aria-label={`Controller for ${role.name}`} value={role.mode} disabled={pending} onChange={e => void assign(role.role_id, e.target.value as Control)}>
          <option value="human">Human</option><option value="deterministic">Built-in AI</option><option value="external">External agent</option>
        </select>
        {role.mode === "external" && <button className="secondary" disabled={pending} onClick={() => void assign(role.role_id, "external")}>Rotate token</button>}
        {role.observation != null && <details><summary>Agent observation</summary><pre>{JSON.stringify(role.observation, null, 2)}</pre></details>}
      </div>)}
      {view.history.length > 0 && <details><summary>AI decision history</summary>{view.history.slice().reverse().map((entry, index) => <div key={index}><p>Tick {entry.tick} · {entry.kind} · role {entry.role_id}</p><pre>{JSON.stringify(entry.details, null, 2)}</pre></div>)}</details>}
    </>}
  </section>;
}
