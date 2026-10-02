import { useEffect, useRef, useState } from "react";
import { ApiError, apiRequest } from "./apiClient";
import type { Role } from "./AuthorityWorkspace";
import type { Projection } from "./globeEntities";
import { usePollingResource } from "./usePollingResource";
import { MissionDebrief } from "./MissionDebrief";

type EngagementBody = { player_id: string; lease_generation: number; intent: {
  intent_id: string; issuer_role: string; target: string; kind: { Engage: { track_id: string } }; requested_tick: number;
} };
type Receipt = { state: string; executed_tick?: number; acknowledged_tick?: number; error?: string };
const receiptText: Record<string, string> = {
  queued: "Engagement queued for delivery", in_transit: "Engagement command in transit",
  awaiting_authority: "Awaiting firing authority", awaiting_execution: "Delivered; awaiting launch",
  awaiting_acknowledgement: "Delivered; awaiting launch confirmation", unconfirmed: "Launch unconfirmed",
  executed: "Weapon launched", rejected: "Engagement rejected", dropped: "Engagement delivery failed",
  expired: "Engagement expired", denied: "Firing authority denied", approved_no_executor: "Approved; no firing platform available"
};

export function CombatOrders({ apiBase, gameId, playerId, role, projection, canIssueOrders, canRecoverOrder, onExecuted, settlingReports = false }: {
  apiBase: string; gameId: string; playerId: string; role: Role; projection: Projection;
  canIssueOrders: boolean; canRecoverOrder: boolean; onExecuted: () => void; settlingReports?: boolean;
}) {
  const controlled = new Set(role.command_units);
  const armed = projection.own_units.filter((unit) => controlled.has(unit.id) && unit.weapon);
  const [showDebrief, setShowDebrief] = useState(false);
  const [unitId, setUnitId] = useState("");
  const [trackId, setTrackId] = useState("");
  const [pending, setPending] = useState(false);
  const [uncertain, setUncertain] = useState<EngagementBody | null>(null);
  const [receiptId, setReceiptId] = useState<string | null>(null);
  const [feedback, setFeedback] = useState("");
  const active = useRef(true);
  const pendingRef = useRef(false);
  const controllerRef = useRef<AbortController | null>(null);
  const observedExecution = useRef<string | null>(null);
  const unit = armed.find((item) => item.id === unitId) ?? armed[0];
  const track = projection.tracks.find((item) => item.track_id === trackId) ?? projection.tracks[0];
  const mission = projection.combat?.mission;
  const finished = mission && mission.status !== "active";
  const eligible = track && unit?.weapon && track.target_side !== role.side && track.identity_confidence >= 0.8
    && track.observed_tick <= projection.tick && projection.tick - track.observed_tick <= unit.weapon.max_track_age_ticks;
  const receiptResource = usePollingResource<Receipt>(receiptId ? `${gameId}:${role.id}:${role.lease_generation}:${receiptId}` : null,
    (signal) => apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intents/${receiptId}?${new URLSearchParams({
      player_id: playerId, lease_generation: String(role.lease_generation)
    })}`, { signal }));
  const receipt = receiptResource.data;

  useEffect(() => {
    active.current = true;
    return () => { active.current = false; controllerRef.current?.abort(); };
  }, []);
  useEffect(() => {
    if (receipt?.state === "executed" && receiptId !== observedExecution.current) {
      observedExecution.current = receiptId;
      onExecuted();
    }
  }, [receipt, receiptId, onExecuted]);

  async function transmit(body: EngagementBody) {
    if (pendingRef.current) return;
    pendingRef.current = true; setPending(true); setFeedback(""); setReceiptId(null);
    const controller = new AbortController(); controllerRef.current = controller;
    const deadline = window.setTimeout(() => controller.abort(), 10_000);
    try {
      await apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intent`, {
        method: "POST", body: JSON.stringify(body), signal: controller.signal
      });
      if (!active.current) return;
      setUncertain(null); setReceiptId(body.intent.intent_id);
    } catch (cause) {
      if (!active.current) return;
      if (cause instanceof ApiError && cause.status >= 400 && cause.status < 500) {
        setUncertain(null); setFeedback(cause.message);
      } else {
        setUncertain(body); setFeedback("The server response was lost. Retry this engagement to check its original submission.");
      }
    } finally {
      window.clearTimeout(deadline); pendingRef.current = false;
      if (active.current) setPending(false);
      if (controllerRef.current === controller) controllerRef.current = null;
    }
  }

  function engage() {
    if (!canIssueOrders || !eligible || !unit || !track || !unit.weapon?.ammunition || finished || pendingRef.current || uncertain) return;
    void transmit({ player_id: playerId, lease_generation: role.lease_generation, intent: {
      intent_id: crypto.randomUUID(), issuer_role: role.id, target: unit.id,
      kind: { Engage: { track_id: track.track_id } }, requested_tick: projection.tick + 1
    } });
  }

  if (!projection.combat) return null;
  const impacts = projection.combat.local_impacts;
  const lastImpact = impacts[impacts.length - 1];
  const received = projection.combat.received_impacts ?? [];
  const lastReceived = received[received.length - 1];
  return <>
    {mission && <section className={`training-mission ${mission.status}`} aria-label="Training mission" aria-live="polite">
      <h2>{mission.status === "succeeded" ? "Mission complete" : mission.status === "failed" ? "Mission failed" : "Mission active"}</h2>
      <p>{mission.title}</p>
      <small>{mission.status === "active" ? `Complete by tick ${mission.deadline_tick}` : `${mission.reason} · tick ${mission.finished_tick}`}</small>
      {finished && <p>{settlingReports ? "Combat has stopped. Final radio reports are still settling." : "The exercise has ended. Leave and create a new game to try again."}</p>}
    </section>}
    <section className="combat-orders" aria-label="Engagement orders">
      <h2>Engagement orders</h2>
      {unit ? <>
        <fieldset disabled={pending || uncertain !== null}>
          <label>Firing platform<select value={unit.id} onChange={(event) => setUnitId(event.target.value)}>
            {armed.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
          </select></label>
          <label>Reported contact<select value={track?.track_id ?? ""} onChange={(event) => setTrackId(event.target.value)}>
            {!projection.tracks.length && <option value="">No contact reports</option>}
            {projection.tracks.map((item, index) => <option key={item.track_id} value={item.track_id}>
              Contact {index + 1} · {Math.round(item.identity_confidence * 100)}% identity · observed tick {item.observed_tick}
            </option>)}
          </select></label>
        </fieldset>
        <p className="combat-ammunition">Training ammunition: {unit.weapon!.ammunition} · range {Math.round(unit.weapon!.range_m / 1000)} km</p>
        <p className="muted">Fires at the reported position. A moving target can escape before impact.</p>
        {!eligible && !finished && <p className="muted">A fresh hostile contact with at least 80% identity confidence is required.</p>}
        <button className="command" disabled={!canIssueOrders || !eligible || !unit.weapon!.ammunition || !!finished || pending || uncertain !== null}
          aria-busy={pending} onClick={engage}>{pending ? "Sending engagement…" : "Send engagement order"}</button>
        {uncertain && <button className="command combat-retry" disabled={pending || !canRecoverOrder} onClick={() => void transmit(uncertain)}>Retry engagement</button>}
        <div role="status" className="combat-feedback" aria-live="polite">
          {pending ? "Submitting engagement…" : feedback || (receipt ? `${receiptText[receipt.state] ?? "Checking engagement"}${receipt.executed_tick !== undefined ? ` at tick ${receipt.executed_tick}` : ""}` : "Orders require firing authority and delivery before launch.")}
          {receipt?.acknowledged_tick !== undefined && <small>Confirmed at radio tick {receipt.acknowledged_tick}</small>}
          {receipt?.error && <small>{receipt.error}</small>}
          {receiptId && receiptResource.status !== "live" && <small>Checking engagement status; reconnecting if necessary.</small>}
        </div>
      </> : <p className="muted">No armed platform under this role's command.</p>}
      {projection.combat.local_shots_in_flight > 0 && <p>Training shots in flight: {projection.combat.local_shots_in_flight}</p>}
      {lastImpact && <p className="combat-impact">Weapon impact: {lastImpact.hit ? "hit" : "miss"} at tick {lastImpact.resolved_tick}</p>}
      {lastReceived && <p className="combat-impact">Received impact report: {lastReceived.report.hit ? "hit" : "miss"} at tick {lastReceived.report.resolved_tick} · received at radio tick {lastReceived.received_tick}</p>}
      <button className="secondary" onClick={() => setShowDebrief(true)}>Mission debrief</button>
    </section>
    {showDebrief && <MissionDebrief apiBase={apiBase} gameId={gameId} playerId={playerId} role={role} units={projection.own_units} onClose={() => setShowDebrief(false)} />}
  </>;
}
