import { useEffect, useRef, useState } from "react";
import { ApiError, apiRequest } from "./apiClient";
import type { Role } from "./AuthorityWorkspace";
import type { Projection } from "./globeEntities";
import { usePollingResource } from "./usePollingResource";
type Command = { lease_generation: number; intent: { intent_id: string; issuer_role: string; target: string; requested_tick: number; kind: { SetRadarEmission: { emitting: boolean } } } };
type Receipt = { state: string; error?: string; acknowledged_tick?: number };
export function RadarControls({ apiBase, gameId, role, projection, enabled, recoverable }: { apiBase: string; gameId: string; role: Role; projection: Projection; enabled: boolean; recoverable: boolean }) {
  const units = projection.own_units.filter(unit => role.command_units.includes(unit.id));
  const [unitId, setUnitId] = useState(""), [command, setCommand] = useState<Command | null>(null), [receiptId, setReceiptId] = useState<string | null>(null), [pending, setPending] = useState(false), [error, setError] = useState("");
  const active = useRef(true), abort = useRef<AbortController | null>(null);
  useEffect(() => { active.current = true; return () => { active.current = false; abort.current?.abort(); }; }, []);
  const receipt = usePollingResource<Receipt>(receiptId ? `${gameId}:${role.id}:${role.lease_generation}:${receiptId}` : null, signal => apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intents/${receiptId}?lease_generation=${role.lease_generation}`, { signal }));
  if (!projection.sensing || !units.length) return null;
  const target = units.find(unit => unit.id === unitId) ?? units[0];
  async function transmit(body: Command) {
    if (pending) return; setPending(true); setError("");
    const controller = new AbortController(); abort.current = controller; const timer = window.setTimeout(() => controller.abort(), 10000);
    try { await apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intent`, { method: "POST", body: JSON.stringify(body), signal: controller.signal }); if (active.current) { setCommand(null); setReceiptId(body.intent.intent_id); } }
    catch (cause) { if (active.current) { setError(cause instanceof Error ? cause.message : String(cause)); setCommand(cause instanceof ApiError && cause.status >= 400 && cause.status < 500 ? null : body); } }
    finally { clearTimeout(timer); if (active.current) setPending(false); }
  }
  function send(emitting: boolean) { if (!enabled || pending || command) return; const body: Command = { lease_generation: role.lease_generation, intent: { intent_id: crypto.randomUUID(), issuer_role: role.id, target: target.id, requested_tick: projection.tick + 1, kind: { SetRadarEmission: { emitting } } } }; void transmit(body); }
  return <section className="radar-controls" aria-label="Radar emissions"><h2>Radar emissions</h2><label>Radar unit<select value={target.id} onChange={event => setUnitId(event.target.value)}>{units.map(unit => <option key={unit.id} value={unit.id}>{unit.name}</option>)}</select></label><p>Last confirmed state: {target.radar_emitting == null ? "Unknown" : target.radar_emitting ? "On" : "Off"}{target.observed_tick != null ? ` · observed tick ${target.observed_tick}` : ""}</p>
    <button disabled={!enabled || pending || !!command} onClick={() => send(true)}>Radar on</button><button disabled={!enabled || pending || !!command} onClick={() => send(false)}>Radar off</button>
    {receipt.data && <p role="status">Radar order: {receipt.data.state.replaceAll("_", " ")}{receipt.data.error ? ` · ${receipt.data.error}` : ""}{receipt.data.acknowledged_tick != null ? ` · confirmation received at tick ${receipt.data.acknowledged_tick}` : ""}</p>}
    {error && <p role="alert">{error}</p>}{command && <button disabled={pending || !recoverable} onClick={() => void transmit(command)}>Retry original radar order</button>}
  </section>;
}
