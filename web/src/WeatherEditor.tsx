import { useEffect, useRef, useState } from "react";
import { apiRequest } from "./apiClient";
import { usePollingResource } from "./usePollingResource";
import type { ObserverRole } from "./observerTypes";
import type { Position, Weather } from "./globeEntities";
export type WeatherRegion = { id: string; center: Position; radius_m: number; priority: number; start_tick: number; end_tick: number | null; conditions: Weather };
type Environment = { revision: number; baseline: Weather; regions: WeatherRegion[]; pending: { effective_tick: number; baseline: Weather; regions: WeatherRegion[] } | null };
const labels: Record<keyof Weather, string> = { visibility_m: "Visibility (m)", cloud_base_m: "Cloud base MSL (m)", cloud_top_m: "Cloud top MSL (m)", cloud_opacity: "Cloud opacity (0–1)", precipitation: "Precipitation (0–1)", thermal_contrast: "Thermal contrast (0–1)" };
export function WeatherEditor({ apiBase, gameId, role, onPreview }: { apiBase: string; gameId: string; role: ObserverRole; onPreview: (regions: WeatherRegion[]) => void }) {
  const resource = usePollingResource<Environment>(`${gameId}:${role.id}:${role.lease_generation}:weather`, signal => apiRequest(apiBase, `/v1/games/${gameId}/environment?${new URLSearchParams({ role_id: role.id, lease_generation: String(role.lease_generation) })}`, { signal }));
  const [baseline, setBaseline] = useState<Weather | null>(null), [regions, setRegions] = useState("[]"), [revision, setRevision] = useState(0), [feedback, setFeedback] = useState(""), [pending, setPending] = useState(false);
  const dirty = useRef(false);
  useEffect(() => { if (resource.data && !dirty.current) { const current = resource.data.pending ?? resource.data; setBaseline(current.baseline); setRegions(JSON.stringify(current.regions, null, 2)); setRevision(resource.data.revision); } }, [resource.data]);
  if (!baseline) return <section aria-label="Weather controls"><h2>Weather</h2><p>{resource.error?.message ?? "Loading conditions…"}</p></section>;
  const controller = role.observer_kind === "controller";
  function preview() { try { const parsed = JSON.parse(regions) as WeatherRegion[]; if (!Array.isArray(parsed)) throw new Error("Regions must be an array"); onPreview(parsed); setFeedback(`Previewing ${parsed.length} weather regions; changes are not applied.`); return parsed; } catch (error) { setFeedback(String(error)); return null; } }
  async function apply() {
    const parsed = preview(); if (!parsed || !controller || pending) return;
    setPending(true);
    try {
      const updated = await apiRequest<Environment>(apiBase, `/v1/games/${gameId}/roles/${role.id}/environment`, { method: "PUT", body: JSON.stringify({ lease_generation: role.lease_generation, expected_revision: revision, baseline, regions: parsed }) });
      dirty.current = false; setRevision(updated.revision); setFeedback(`Weather revision ${updated.revision} committed; effective at tick ${updated.pending?.effective_tick}.`); resource.refresh();
    } catch (error) { setFeedback(String(error)); resource.refresh(); } finally { setPending(false); }
  }
  return <section className="weather-editor" aria-label="Weather controls"><h2>{controller ? "Controller weather" : "Weather — read only"}</h2><p>Revision {resource.data?.revision} {resource.data?.pending && `· pending at tick ${resource.data.pending.effective_tick}`}</p>
    <fieldset disabled={!controller || pending}>{(Object.keys(labels) as (keyof Weather)[]).map(key => <label key={key}>{labels[key]}<input type="number" step="any" value={baseline[key]} onChange={event => { dirty.current = true; setBaseline({ ...baseline, [key]: Number(event.target.value) }); }} /></label>)}
      <label>Weather regions (JSON)<textarea rows={7} value={regions} onChange={event => { dirty.current = true; setRegions(event.target.value); }} /></label>
    </fieldset>
    <button className="secondary" onClick={preview}>Preview weather</button>{controller && <><button className="command" disabled={pending} onClick={() => void apply()}>Apply weather</button><button className="secondary" onClick={() => { dirty.current = false; resource.refresh(); }}>Reload latest revision</button></>}
    {feedback && <p role="status">{feedback}</p>}
  </section>;
}
