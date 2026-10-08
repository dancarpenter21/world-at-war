import { authenticatedFetch } from "./apiClient";
import { useState } from "react";
import type { Role } from "./AuthorityWorkspace";
import type { ActivationPeriod, PlanningView } from "./PlanningWorkspace";

type Resolution = { exclude?: boolean; controller_role_id?: string; kind?: string; floor_m?: number; ceiling_m?: number; periods?: ActivationPeriod[] };
type ImportRecord = { external_id: string; name: string; line: number; issues: string[]; warnings: string[]; excluded: boolean };
type Preview = { valid: boolean; expected_revision: number; issues: string[]; records: ImportRecord[]; changes: { external_id: string; action: string }[] };

export function AirspaceImport({ base, playerId, role, roles, revision, dirty, busy, onApplied, onBusyChange }: {
  base: string; playerId: string; role: Role; roles: Role[]; revision: number; dirty: boolean; busy: boolean;
  onApplied: (view: PlanningView) => void; onBusyChange: (busy: boolean) => void;
}) {
  const [source, setSource] = useState("");
  const [anchorUtc, setAnchorUtc] = useState("");
  const [anchorTick, setAnchorTick] = useState(0);
  const [year, setYear] = useState(new Date().getUTCFullYear());
  const [horizon, setHorizon] = useState("");
  const [resolutions, setResolutions] = useState<Record<string, Resolution>>({});
  const [periodText, setPeriodText] = useState<Record<string, string>>({});
  const [preview, setPreview] = useState<Preview | null>(null);
  const [previewKey, setPreviewKey] = useState("");
  const [error, setError] = useState("");
  const [feedback, setFeedback] = useState("");
  const payload = { player_id: playerId, lease_generation: role.lease_generation, expected_revision: revision, source,
    options: { anchor_utc: anchorUtc, anchor_tick: anchorTick, year, horizon_end_utc: horizon, resolutions } };
  const key = JSON.stringify(payload);
  const periodsValid = Object.entries(periodText).every(([id, text]) => resolutions[id]?.exclude || !text.trim() || /^\d+\s*-\s*\d+(\s*,\s*\d+\s*-\s*\d+)*$/.test(text.trim()));
  const ready = Boolean(source.trim() && anchorUtc.trim() && horizon.trim() && periodsValid && !dirty && !busy);
  const current = previewKey === key;

  function resolve(id: string, change: Partial<Resolution>) {
    setResolutions(previous => ({ ...previous, [id]: { ...previous[id], ...change } }));
    setFeedback("");
  }
  function changeSource(value: string) {
    setSource(value); setPreview(null); setPreviewKey(""); setResolutions({}); setPeriodText({}); setFeedback(""); setError("");
  }
  async function submit(action: "preview" | "apply") {
    onBusyChange(true); setError(""); setFeedback("");
    try {
      const response = await authenticatedFetch(`${base}/roles/${role.id}/planning/aco/${action}`, {
        method: "POST", credentials: "include", headers: { "content-type": "application/json" }, body: key
      });
      const body = await response.json();
      if (!response.ok) throw new Error(body.error ?? "Airspace import rejected");
      if (action === "preview") { setPreview(body as Preview); setPreviewKey(key); }
      else { onApplied(body as PlanningView); setPreviewKey(""); setFeedback("Airspaces applied to draft. Publish the campaign to deliver them."); }
    } catch (e) { setError((e as Error).message); setPreviewKey(""); }
    finally { onBusyChange(false); }
  }
  return <details className="airspace-import">
    <summary>Import airspace order</summary>
    <section aria-label="Airspace order import">
      <p>Import an ACO into the saved draft. Specify how UTC dates map to simulation ticks, then preview and resolve each record.</p>
      <fieldset disabled={busy}>
        <label>ACO text file<input type="file" accept=".aco,.txt,text/plain" onChange={event => {
          const file = event.target.files?.[0];
          if (file) void file.text().then(changeSource).catch(() => setError("Could not read the selected file."));
        }} /></label>
        <label>ACO source<textarea value={source} onChange={event => changeSource(event.target.value)} /></label>
        <div className="planning-row">
          <label>UTC anchor<input placeholder="2026-09-01T00:00:00Z" value={anchorUtc} onChange={event => setAnchorUtc(event.target.value)} /></label>
          <label>Anchor tick<input type="number" min="0" value={anchorTick} onChange={event => setAnchorTick(Number(event.target.value))} /></label>
          <label>ACO year<input type="number" value={year} onChange={event => setYear(Number(event.target.value))} /></label>
          <label>UTC horizon<input placeholder="2026-09-03T00:00:00Z" value={horizon} onChange={event => setHorizon(event.target.value)} /></label>
        </div>
        {preview?.issues.map((issue, index) => <p key={index} role="alert">{issue}</p>)}
        {preview?.records.map(record => <fieldset key={record.external_id} aria-label={`Resolve ${record.external_id}`}>
          <legend>{record.name} · line {record.line}</legend>
          {[...record.issues, ...record.warnings].map((issue, index) => <p key={index}>{issue}</p>)}
          <label><input type="checkbox" checked={resolutions[record.external_id]?.exclude ?? false} onChange={event => resolve(record.external_id, { exclude: event.target.checked })} />Exclude {record.external_id}</label>
          <div className="planning-row">
            <label>Assigned controller<select value={resolutions[record.external_id]?.controller_role_id ?? ""} onChange={event => resolve(record.external_id, { controller_role_id: event.target.value || undefined })}><option value="">Choose a controller</option>{roles.filter(candidate => candidate.side === role.side).map(candidate => <option key={candidate.id} value={candidate.id}>{candidate.name}</option>)}</select></label>
            <label>Airspace kind<select value={resolutions[record.external_id]?.kind ?? ""} onChange={event => resolve(record.external_id, { kind: event.target.value || undefined })}><option value="">Use source kind</option>{["sector", "corridor", "restricted", "patrol"].map(kind => <option key={kind}>{kind}</option>)}</select></label>
            {(["floor_m", "ceiling_m"] as const).map(bound => <label key={bound}>{bound === "floor_m" ? "Resolved floor MSL (m)" : "Resolved ceiling MSL (m)"}<input type="number" step="any" value={resolutions[record.external_id]?.[bound] ?? ""} onChange={event => resolve(record.external_id, { [bound]: event.target.value === "" ? undefined : Number(event.target.value) })} /></label>)}
            <label>Resolved activation ticks<input placeholder="0-600, 900-1200" value={periodText[record.external_id] ?? ""} onChange={event => {
              const text = event.target.value; setPeriodText(previous => ({ ...previous, [record.external_id]: text }));
              const periods = /^\d+\s*-\s*\d+(\s*,\s*\d+\s*-\s*\d+)*$/.test(text.trim()) ? text.split(",").map(pair => { const [start_tick, end_tick] = pair.split("-").map(Number); return { start_tick, end_tick }; }) : undefined;
              resolve(record.external_id, { periods });
            }} /></label>
          </div>
        </fieldset>)}
        {!periodsValid && <p role="alert">Use tick ranges such as 0-600, 900-1200.</p>}
        {preview && <div aria-label="Proposed airspace changes">{preview.changes.map(change => <p key={change.external_id}>{change.external_id}: {change.action}</p>)}<p>{current ? preview.valid ? "Preview valid." : "Resolve or exclude invalid records, then preview again." : "Preview outdated. Preview again before applying."}</p></div>}
        {dirty && <p>Save the campaign draft before importing.</p>}
        <div className="planning-row"><button disabled={!ready} onClick={() => void submit("preview")}>Preview airspace import</button><button disabled={!ready || !current || !preview?.valid} onClick={() => void submit("apply")}>Apply airspaces to draft</button></div>
      </fieldset>
      {error && <p role="alert">{error}</p>}{feedback && <p role="status">{feedback}</p>}
    </section>
  </details>;
}
