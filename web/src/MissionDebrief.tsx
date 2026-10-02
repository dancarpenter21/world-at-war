import { useEffect, useRef } from "react";
import { apiRequest } from "./apiClient";
import type { Role } from "./AuthorityWorkspace";
import type { CombatProjection, Unit } from "./globeEntities";
import { usePollingResource } from "./usePollingResource";

type RadioLeg = { profile_id: string; state: string; sent_tick: number; delivered_tick: number | null };
type Entry = { intent_id: string; platform_id: string; submitted_tick: number | null; observed_tick: number | null; approval_ticks: number[];
  launch_tick: number | null; acknowledged_tick: number | null; impact_tick: number | null; report_received_tick: number | null; hit: boolean | null; state: string; error: string | null; radio_legs: RadioLeg[] };
type Debrief = { tick: number; radio_tick: number; settling_reports: boolean; mission: CombatProjection["mission"]; entries: Entry[] };
const tick = (value: number | null) => value === null ? "—" : String(value);
const profileName = (profile: string) => profile.includes("network.ack") ? "Execution acknowledgement" : profile.includes("impact-report") ? "Impact report" : profile.includes("authority") ? "Authority request" : "Firing order";

export function MissionDebrief({ apiBase, gameId, playerId, role, units, onClose }: {
  apiBase: string; gameId: string; playerId: string; role: Role; units: Unit[]; onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const resource = usePollingResource<Debrief>(`${gameId}:${role.id}:${role.lease_generation}`,
    (signal) => apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/debrief?${new URLSearchParams({ player_id: playerId, lease_generation: String(role.lease_generation) })}`, { signal }));
  useEffect(() => { const element = dialog.current; element?.showModal(); return () => element?.close(); }, []);
  const data = resource.data;
  return <dialog ref={dialog} className="mission-debrief" aria-labelledby="debrief-title" onCancel={(event) => { event.preventDefault(); onClose(); }}>
    <header><h1 id="debrief-title">Mission debrief</h1><button className="secondary" autoFocus onClick={onClose}>Close debrief</button></header>
    <p>Only events known to your role are shown. A launch receipt is not a hit report.</p>
    {resource.status !== "live" && <p role="status">{data ? "Keeping the last debrief while reconnecting." : "Loading mission timeline…"} {resource.error?.message}<button className="secondary" onClick={resource.refresh}>Retry debrief</button></p>}
    {data && <>
      <p className="debrief-outcome">{data.mission?.title} · {data.mission?.status.replaceAll("_", " ")}
        {data.mission?.finished_tick !== null && data.mission?.finished_tick !== undefined && ` at tick ${data.mission.finished_tick}`}</p>
      {data.settling_reports && <p role="status">Combat is stopped. Final radio reports are still settling.</p>}
      <p className="muted">Combat clock: {data.tick} · Radio clock: {data.radio_tick}</p>
      {!data.entries.length ? <p>No engagement orders are known to this role.</p> : <div className="debrief-table-wrap"><table>
        <caption>Engagement timeline</caption><thead><tr><th scope="col">Order</th><th scope="col">Observed</th><th scope="col">Approved</th><th scope="col">Delivered</th><th scope="col">Launched</th><th scope="col">Confirmed</th><th scope="col">Impact</th><th scope="col">Report received</th><th scope="col">Result</th></tr></thead>
        <tbody>{data.entries.map((entry, index) => {
          const firing = entry.radio_legs.find((leg) => leg.profile_id.includes("engage-order"));
          return <tr key={entry.intent_id} data-intent-id={entry.intent_id}><th scope="row">Shot {index + 1}<small>{units.find((unit) => unit.id === entry.platform_id)?.name ?? "Firing platform"}</small></th>
            <td>{tick(entry.observed_tick)}</td><td>{entry.approval_ticks.length ? entry.approval_ticks.join(", ") : "—"}</td><td>{tick(firing?.delivered_tick ?? null)}</td><td>{tick(entry.launch_tick)}</td><td>{tick(entry.acknowledged_tick)}</td><td>{tick(entry.impact_tick)}</td><td>{tick(entry.report_received_tick)}</td>
            <td>{entry.hit === null ? entry.state.replaceAll("_", " ") : entry.hit ? "Hit" : "Miss"}{entry.hit === null && entry.launch_tick !== null && <small>Impact result not received</small>}{entry.error && <small>{entry.error}</small>}</td></tr>;
        })}</tbody>
      </table></div>}
      {data.entries.map((entry, index) => <section className="debrief-radio" key={entry.intent_id} aria-label={`Shot ${index + 1} radio history`}>
        <h2>Shot {index + 1} · submitted tick {tick(entry.submitted_tick)}</h2>
        <ul>{entry.radio_legs.map((leg, legIndex) => <li key={legIndex}>{profileName(leg.profile_id)} · sent {leg.sent_tick} · {leg.state}{leg.delivered_tick !== null && ` at radio tick ${leg.delivered_tick}`}</li>)}</ul>
      </section>)}
    </>}
  </dialog>;
}
