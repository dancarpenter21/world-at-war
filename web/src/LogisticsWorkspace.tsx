import { useEffect, useRef, useState } from "react";
import { ApiError, apiRequest } from "./apiClient";
import type { Role } from "./AuthorityWorkspace";
import type { Projection } from "./globeEntities";
import { usePollingResource } from "./usePollingResource";
import { transferTimeSeconds, validCargo, type Interface, type LogisticsCommand, type LogisticsProjection } from "./logisticsModel";
import fuelCatalog from "../../data/logistics/aircraft-fuel.v1.json";

type Body = { player_id: string; lease_generation: number; intent: { intent_id: string; issuer_role: string; target: string; kind: { Logistics: { command: LogisticsCommand } }; requested_tick: number } };
type Receipt = { state: string; error?: string; executed_tick?: number };
export function LogisticsStatus({ logistics, names, tick }: { logistics: LogisticsProjection; names: Map<string, string>; tick: number }) {
  return <div className="logistics-status">
    {logistics.units.map(u => <article className="planning-card" key={u.unit_id}><strong>{names.get(u.unit_id) ?? u.profile_id}</strong><p>{u.phase.replaceAll("_", " ")} · {u.fuel_kg.toFixed(1)} / {u.capacity_kg.toFixed(0)} kg fuel · {u.ammunition} stores</p><p>{u.detail}</p>{u.estimated_endurance_seconds != null && <p>Estimated endurance at current burn: {Math.round(u.estimated_endurance_seconds)} game seconds</p>}{(u.rate_kg_s > 0 || u.transferred_kg > 0) && <p>{u.rate_kg_s.toFixed(2)} kg/s · {u.transferred_kg.toFixed(1)} kg transferred</p>}{u.cargo.fuel_kg + u.cargo.ammunition + u.cargo.repair_points > 0 && <p>Cargo: {u.cargo.fuel_kg.toFixed(1)} kg fuel · {u.cargo.ammunition} stores · {u.cargo.repair_points.toFixed(0)} repair points</p>}<small>Observed tick {u.observed_tick} · received tick {u.received_tick} · observation age {tick - u.observed_tick}s</small></article>)}
    {logistics.facilities.map(({ facility: f, reserved, observed_tick, received_tick }) => <article className="planning-card" key={f.unit_id}><strong>{names.get(f.unit_id) ?? f.kind}</strong><p>{f.stock.fuel_kg.toFixed(1)} kg available fuel · {f.stock.ammunition} stores · {f.stock.repair_points.toFixed(0)} repair points</p><p>{reserved?.fuel_kg.toFixed(1) ?? "0"} kg fuel reserved · {f.service_slots} service slots · {f.parking_capacity} parking/berth capacity</p><small>Observed tick {observed_tick} · received tick {received_tick}</small></article>)}
    {logistics.unavailable.map(u => <article key={u.unit_id}><strong>{names.get(u.unit_id) ?? u.profile_id}: logistics unavailable</strong><p>{u.reasons.join("; ")}</p></article>)}
  </div>;
}
export function LogisticsWorkspace({ apiBase, gameId, playerId, role, projection, enabled, recoverable }: { apiBase: string; gameId: string; playerId: string; role: Role; projection: Projection; enabled: boolean; recoverable: boolean }) {
  const data = projection.logistics;
  const [target, setTarget] = useState(""), [facility, setFacility] = useState(""), [source, setSource] = useState(""), [donor, setDonor] = useState(""), [iface, setInterface] = useState<Interface>("boom");
  const [amount, setAmount] = useState("100"), [ammo, setAmmo] = useState("0"), [repair, setRepair] = useState("0");
  const [pending, setPending] = useState(false), [uncertain, setUncertain] = useState<Body | null>(null), [receiptId, setReceiptId] = useState<string | null>(null), [feedback, setFeedback] = useState("");
  const busy = useRef(false), active = useRef(true), request = useRef<AbortController | null>(null);
  useEffect(() => { active.current = true; return () => { active.current = false; request.current?.abort(); }; }, []);
  const receipt = usePollingResource<Receipt>(receiptId ? `${gameId}:${role.id}:${role.lease_generation}:${receiptId}` : null, signal => apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intents/${receiptId}?${new URLSearchParams({ player_id: playerId, lease_generation: String(role.lease_generation) })}`, { signal }));
  if (!data) return null;
  const controlled = data.units.filter(u => role.command_units.includes(u.unit_id));
  const selected = controlled.find(u => u.unit_id === target) ?? controlled[0];
  const names = new Map(projection.own_units.map(u => [u.id, u.name]));
  const destinations=data.facilities.filter(({facility:f})=>!selected || f.compatible_profile_ids.includes(selected.profile_id) || (f.kind==="carrier"&&selected.support_interfaces.includes("underway")));
  const sources=data.facilities.filter(({facility:f})=>!selected || f.compatible_profile_ids.includes(selected.profile_id));
  const donors=data.units.filter(u=>u.unit_id!==selected?.unit_id&&u.support_interfaces.includes(iface));
  const selectedFacility = destinations.find(f => f.facility.unit_id === facility)?.facility.unit_id ?? destinations[0]?.facility.unit_id ?? "";
  const selectedSource = sources.find(f => f.facility.unit_id === source)?.facility.unit_id ?? sources[0]?.facility.unit_id ?? "";
  const selectedDonor = donors.find(u => u.unit_id === donor)?.unit_id ?? donors[0]?.unit_id ?? "";
  async function transmit(body: Body) {
    if (busy.current) return;
    busy.current = true; setPending(true); setFeedback(""); setReceiptId(null);
    const controller = new AbortController(); request.current = controller;
    const deadline = window.setTimeout(() => controller.abort(), 10000);
    try {
      await apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intent`, { method: "POST", body: JSON.stringify(body), signal: controller.signal });
      if (active.current) { setUncertain(null); setReceiptId(body.intent.intent_id); }
    } catch (error) {
      if (active.current) {
        if (error instanceof ApiError && error.status >= 400 && error.status < 500) { setUncertain(null); setFeedback(error.message); }
        else { setUncertain(body); setFeedback("Submission response lost. Retry the same order to recover its receipt."); }
      }
    } finally { window.clearTimeout(deadline); busy.current = false; if (active.current) setPending(false); }
  }
  function send(command: LogisticsCommand) {
    if (!enabled || !selected || uncertain || busy.current) return;
    if (command.action === "refuel" && (!Number.isFinite(command.amount_kg) || command.amount_kg <= 0)) { setFeedback("Enter a positive fuel quantity."); return; }
    if (command.action === "cargo" && !validCargo(command.manifest)) { setFeedback("Enter nonnegative cargo quantities with at least one positive quantity."); return; }
    void transmit({ player_id: playerId, lease_generation: role.lease_generation, intent: { intent_id: crypto.randomUUID(), issuer_role: role.id, target: selected.unit_id, kind: { Logistics: { command } }, requested_tick: projection.tick + 1 } });
  }
  const disabled = !enabled || !selected || pending || uncertain !== null;
  const duration = transferTimeSeconds(Number(amount), selected?.rate_kg_s ?? 0);
  return <details className="logistics-workspace" open><summary>Logistics and basing</summary>
    {controlled.length > 0 && <label>Logistics unit<select aria-label="Logistics unit" disabled={pending || uncertain !== null} value={selected?.unit_id ?? ""} onChange={e => setTarget(e.target.value)}>{controlled.map(u => <option key={u.unit_id} value={u.unit_id}>{names.get(u.unit_id) ?? u.profile_id}</option>)}</select></label>}
    <LogisticsStatus logistics={{ units: selected ? [selected] : [], facilities: controlled.length ? [] : data.facilities, unavailable: data.unavailable }} names={names} tick={projection.tick} />
    {controlled.length > 0 && <details><summary>Facility stocks · {data.facilities.length} received reports</summary><LogisticsStatus logistics={{ units: [], facilities: data.facilities, unavailable: [] }} names={names} tick={projection.tick} /></details>}
    {data.units.length > 1 && <details><summary>All unit reports · {data.units.length}</summary><LogisticsStatus logistics={{ units: data.units, facilities: [], unavailable: [] }} names={names} tick={projection.tick} /></details>}
    {controlled.length > 0 && <fieldset disabled={disabled}>
      <label>Recovery / cargo destination<select aria-label="Logistics destination" value={selectedFacility} onChange={e => setFacility(e.target.value)}>{destinations.map(({ facility: f }) => <option key={f.unit_id} value={f.unit_id}>{names.get(f.unit_id) ?? f.kind}</option>)}</select></label>
      <button className="command" disabled={!sources.some(f=>f.facility.unit_id===selectedFacility)} onClick={() => send({ action: "return", facility_id: selectedFacility })}>Return and service</button>
      <button className="secondary" onClick={() => send({ action: "depart" })}>Depart / launch early</button>
      <label>Refueling interface<select aria-label="Refueling interface" value={iface} onChange={e => setInterface(e.target.value as Interface)}><option value="boom">Boom</option><option value="drogue">Probe and drogue</option><option value="underway">Underway replenishment</option></select></label>
      <button className="command" disabled={!selected?.support_interfaces.includes(iface)} onClick={() => send({ action: "support", interface: iface })}>Establish refueling support</button>
      <label>Assigned donor<select aria-label="Assigned donor" value={selectedDonor} onChange={e => setDonor(e.target.value)}>{donors.map(u => <option key={u.unit_id} value={u.unit_id}>{names.get(u.unit_id) ?? u.profile_id}</option>)}</select></label>
      <label>Fuel quantity (kg)<input aria-label="Fuel quantity (kg)" type="number" min="0" step="any" value={amount} onChange={e => setAmount(e.target.value)} /></label>
      <button className="command" disabled={!selectedDonor || !selected?.receiving_interfaces.includes(iface)} onClick={() => send({ action: "refuel", donor_id: selectedDonor, interface: iface, amount_kg: Number(amount) })}>Request fuel transfer</button>
      {duration !== null && <p>At the current transfer rate: {duration.toFixed(1)} game seconds for this quantity.</p>}
      <label>Cargo source<select aria-label="Cargo source" value={selectedSource} onChange={e => setSource(e.target.value)}>{sources.map(({ facility: f }) => <option key={f.unit_id} value={f.unit_id}>{names.get(f.unit_id) ?? f.kind}</option>)}</select></label>
      <label>Cargo stores<input type="number" min="0" step="1" value={ammo} onChange={e => setAmmo(e.target.value)} /></label>
      <label>Cargo repair points<input type="number" min="0" step="any" value={repair} onChange={e => setRepair(e.target.value)} /></label>
      <button className="command" disabled={!selectedSource || !selectedFacility || selectedSource === selectedFacility} onClick={() => send({ action: "cargo", source_id: selectedSource, destination_id: selectedFacility, manifest: { fuel_kg: Number(amount), ammunition: Number(ammo), repair_points: Number(repair) } })}>Dispatch cargo mission</button>
      <button className="secondary" onClick={() => send({ action: "cancel" })}>Cancel logistics task</button>
    </fieldset>}
    {uncertain && <button disabled={!recoverable || pending} onClick={() => void transmit(uncertain)}>Retry logistics order</button>}
    <p role="status">{pending ? "Submitting logistics order…" : feedback || (receipt.data ? `Order ${receipt.data.state.replaceAll("_", " ")}${receipt.data.error ? `: ${receipt.data.error}` : ""}. Task completion is reported separately.` : receipt.error?.message ?? "")}</p>
    <details><summary>Aircraft fuel sources and coverage</summary>{fuelCatalog.aircraft.map(a => <article key={a.id}><strong>{a.display_name}</strong>{a.facts.map((f, i) => <p key={i}>{f.field.replaceAll("_", " ")}: {f.value.toLocaleString()} {f.unit} · <a href={f.source_url} target="_blank" rel="noreferrer">Source</a></p>)}<p>Unavailable: {a.missing.join("; ")}</p></article>)}</details>
  </details>;
}
