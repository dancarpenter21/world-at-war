import { useEffect, useRef, useState } from "react";
import { Cartesian3, Color, EllipsoidTerrainProvider, ImageryLayer, OpenStreetMapImageryProvider, Viewer } from "cesium";
import "cesium/Build/Cesium/Widgets/widgets.css";
import { ApiError, apiRequest } from "./apiClient";
import { usePollingResource, type PollingStatus } from "./usePollingResource";
import type { ObserverRole, TruthProjection } from "./observerTypes";
import { updateTruthEntities } from "./truthEntities";
import { attachMapKeyboardControls } from "./mapKeyboardControls";

export function ObserverWorkspace({ apiBase, gameId, role, onAccessEnded, onStatus }: { apiBase: string; gameId: string; role: ObserverRole; onAccessEnded: () => void; onStatus: (status: PollingStatus, tick: number | null) => void }) {
  const truth = usePollingResource<TruthProjection>(`${gameId}:${role.id}:${role.lease_generation}`, signal => apiRequest(apiBase,
    `/v1/games/${gameId}/truth?${new URLSearchParams({ role_id: role.id, lease_generation: String(role.lease_generation) })}`, { signal }));
  const statusCallback = useRef(onStatus); statusCallback.current = onStatus;
  useEffect(() => { statusCallback.current(truth.status, truth.data?.tick ?? null); }, [truth.status, truth.data?.tick]);
  const [network, setNetwork] = useState(true);
  const [selected, setSelected] = useState("");
  const ended = useRef(onAccessEnded); ended.current = onAccessEnded;
  useEffect(() => {
    if (truth.error instanceof ApiError && [401, 403, 404].includes(truth.error.status)) ended.current();
  }, [truth.error]);
  const unit = truth.data?.units.find(u => `unit:${u.id}` === selected);
  const weapon = truth.data?.weapons.find(w => `weapon:${w.id}` === selected);
  const names = new Map(truth.data?.units.map(u => [u.id, u.name]));
  truth.data?.weapons.forEach(w => names.set(w.id, `Weapon ${w.id.slice(-6)}`));
  return <section className="truth-workspace" aria-label="Ground truth workspace">
    <aside className="truth-inspector"><h1>Ground truth — read only</h1><p>{role.name} · tick {truth.data?.tick ?? "—"}</p>
      <p role="status">{truth.status === "live" ? "Current simulation state" : truth.status === "reconnecting" ? "Connection interrupted — last received state" : "Connecting…"}</p>
      {truth.error && <p role="alert">{truth.error.message}</p>}
      <label className="toggle"><input type="checkbox" checked={network} onChange={e => setNetwork(e.target.checked)} /> Show network links</label>
      <h2>Object inspector</h2>
      <select aria-label="Inspect truth object" value={unit || weapon ? selected : ""} onChange={e => setSelected(e.target.value)}>
        <option value="">Select a unit or weapon</option>
        {truth.data?.units.map(u => <option key={u.id} value={`unit:${u.id}`}>{u.side} · {u.name}</option>)}
        {truth.data?.weapons.map(w => <option key={w.id} value={`weapon:${w.id}`}>{w.kind} · {w.phase}</option>)}
      </select>
      {unit && <dl><dt>Name</dt><dd>{unit.name}</dd><dt>Side</dt><dd>{unit.side}</dd><dt>Position</dt><dd>{unit.position.latitude_deg.toFixed(4)}, {unit.position.longitude_deg.toFixed(4)} · {unit.position.altitude_m.toFixed(0)} m</dd><dt>Damage</dt><dd>{unit.destroyed ? "Destroyed" : "Active"}{unit.hit_points == null ? "" : ` · ${unit.hit_points} HP`}</dd><dt>Ammunition</dt><dd>{unit.ammunition}</dd><dt>Fuel (modeled seconds)</dt><dd>{unit.fuel.toFixed(0)}</dd></dl>}
      {weapon && <dl><dt>Model</dt><dd>{weapon.kind}</dd><dt>State</dt><dd>{weapon.phase}</dd><dt>Position</dt><dd>{weapon.position ? `${weapon.position.latitude_deg.toFixed(4)}, ${weapon.position.longitude_deg.toFixed(4)}` : "No flight trajectory modeled"}</dd><dt>Impact tick</dt><dd>{weapon.impact_tick ?? "Not scheduled"}</dd><dt>Provider</dt><dd>{weapon.provider_id ? names.get(weapon.provider_id) ?? weapon.provider_id : "None"}</dd></dl>}
      <h2>Network topology</h2><p>Current links and queues. Message contents are not included.</p>
      <div className="truth-links"><table><thead><tr><th>Link</th><th>State</th><th>Queue</th></tr></thead><tbody>
        {truth.data?.communication_links.map(l => <tr key={l.id}><td>{names.get(l.from_entity_id) ?? l.from_entity_id} → {names.get(l.to_entity_id) ?? l.to_entity_id}</td><td>{l.available ? "Available" : "Unavailable"}{l.jammed > 0 ? " · jammed" : ""}</td><td>{l.queued_packets} packets / {l.queued_bytes} B</td></tr>)}
      </tbody></table></div>
    </aside>
    {truth.data && <TruthMap truth={truth.data} network={network} onSelect={setSelected} />}
  </section>;
}
function TruthMap({ truth, network, onSelect }: { truth: TruthProjection; network: boolean; onSelect: (id: string) => void }) {
  const host = useRef<HTMLDivElement>(null), viewer = useRef<Viewer | null>(null);
  const selection = useRef(onSelect); selection.current = onSelect;
  useEffect(() => {
    const map = new Viewer(host.current!, {
      animation: false, baseLayerPicker: false, timeline: false, geocoder: false, homeButton: false,
      navigationHelpButton: false, sceneModePicker: false, fullscreenButton: false, infoBox: false,
      terrainProvider: new EllipsoidTerrainProvider(), requestRenderMode: true, maximumRenderTimeChange: Infinity, targetFrameRate: 30,
      baseLayer: new ImageryLayer(new OpenStreetMapImageryProvider({ url: "https://tile.openstreetmap.org/" })),
    });
    map.scene.globe.baseColor = Color.fromCssColorString("#1f3340");
    const first = truth.units[0]?.position;
    map.camera.setView({ destination: Cartesian3.fromDegrees(first?.longitude_deg ?? -40, first?.latitude_deg ?? 30, first ? 220000 : 20000000) });
    const controls = attachMapKeyboardControls(map.camera, map.scene.globe.ellipsoid); controls.setEnabled(true);
    const stop = map.selectedEntityChanged.addEventListener(entity => selection.current(entity?.id ?? ""));
    viewer.current = map;
    return () => { stop(); controls.destroy(); viewer.current = null; map.destroy(); };
  }, []);
  useEffect(() => {
    if (!viewer.current) return;
    updateTruthEntities(viewer.current.entities, truth, network); viewer.current.scene.requestRender();
  }, [truth, network]);
  return <div className="truth-map globe" ref={host} aria-label="Ground truth map" />;
}
