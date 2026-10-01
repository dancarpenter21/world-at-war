import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import type { Role } from "./AuthorityWorkspace";
import type { Projection } from "./globeEntities";
import { ApiError, apiRequest } from "./apiClient";
import { movementDescription, movementVector, type MovementVector } from "./movement";
import { usePollingResource } from "./usePollingResource";

type MovementIntent = {
  intent_id: string; issuer_role: string; target: string;
  kind: { Move: MovementVector }; requested_tick: number;
};
type OrderBody = { player_id: string; lease_generation: number; intent: MovementIntent };
type Submission = { status: "queued" | "pending_authority"; message_id: string; request_id?: string };
type IntentState = "queued" | "in_transit" | "awaiting_authority" | "awaiting_execution" | "executed" | "rejected" | "dropped" | "expired" | "denied" | "approved_no_executor";
type Receipt = { intent: MovementIntent; state: IntentState; executed_tick?: number; error?: string };
const statusText: Record<IntentState, string> = {
  queued: "Queued for delivery", in_transit: "Command in transit", awaiting_authority: "Awaiting authority approval",
  awaiting_execution: "Delivered; awaiting execution", executed: "Order executed", rejected: "Order rejected",
  dropped: "Command delivery failed", expired: "Command expired", denied: "Authority denied the order",
  approved_no_executor: "Approved; no executor available"
};

export function MovementOrders({ apiBase, gameId, playerId, role, projection, canIssueOrders, canRecoverOrder, onExecuted }: {
  apiBase: string; gameId: string; playerId: string; role: Role; projection: Projection;
  canIssueOrders: boolean; canRecoverOrder: boolean; onExecuted?: () => void;
}) {
  const units = useMemo(() => {
    const controlled = new Set(role.command_units);
    return projection.own_units.filter((unit) => controlled.has(unit.id));
  }, [projection.own_units, role.command_units]);
  const [targetId, setTargetId] = useState(() => units.find((unit) => unit.domain === "Air")?.id ?? units[0]?.id ?? "");
  const [course, setCourse] = useState("0");
  const [speed, setSpeed] = useState("130");
  const [pending, setPending] = useState(false);
  const [uncertainOrder, setUncertainOrder] = useState<OrderBody | null>(null);
  const [feedback, setFeedback] = useState("");
  const [receiptId, setReceiptId] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const requestRef = useRef<AbortController | null>(null);
  const activeRef = useRef(true);
  const target = units.find((unit) => unit.id === targetId) ?? units.find((unit) => unit.domain === "Air") ?? units[0];
  const receiptResource = usePollingResource<Receipt>(receiptId ? `${gameId}:${role.id}:${role.lease_generation}:${receiptId}` : null,
    (signal) => apiRequest(apiBase, `/v1/games/${gameId}/roles/${role.id}/intents/${receiptId}?${new URLSearchParams({
      player_id: playerId, lease_generation: String(role.lease_generation)
    })}`, { signal }));

  useEffect(() => {
    activeRef.current = true;
    return () => { activeRef.current = false; requestRef.current?.abort(); };
  }, []);

  const transmit = async (body: OrderBody) => {
    if (pendingRef.current) return;
    pendingRef.current = true; setPending(true); setFeedback(""); setReceiptId(null);
    const controller = new AbortController(); requestRef.current = controller;
    const deadline = window.setTimeout(() => controller.abort(), 10_000);
    try {
      await apiRequest<Submission>(apiBase, `/v1/games/${gameId}/roles/${role.id}/intent`, {
        method: "POST", body: JSON.stringify(body), signal: controller.signal
      });
      if (!activeRef.current) return;
      setUncertainOrder(null); setReceiptId(body.intent.intent_id);
    } catch (cause) {
      if (!activeRef.current) return;
      if (cause instanceof ApiError && cause.status >= 400 && cause.status < 500) {
        setUncertainOrder(null); setFeedback(cause.message);
      } else {
        setUncertainOrder(body);
        setFeedback("The server response was lost. Retry this order to check its original submission.");
      }
    } finally {
      window.clearTimeout(deadline);
      pendingRef.current = false;
      if (activeRef.current) setPending(false);
      if (requestRef.current === controller) requestRef.current = null;
    }
  };

  const send = (nextCourse: number, nextSpeed: number) => {
    if (!canIssueOrders || !target || pendingRef.current || uncertainOrder) return;
    try {
      const vector = movementVector(nextCourse, nextSpeed);
      void transmit({ player_id: playerId, lease_generation: role.lease_generation, intent: {
        intent_id: crypto.randomUUID(), issuer_role: role.id, target: target.id,
        kind: { Move: vector }, requested_tick: projection.tick + 1
      } });
    } catch (cause) { setFeedback(cause instanceof Error ? cause.message : String(cause)); }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!course.trim() || !speed.trim()) { setFeedback("Enter a course and speed."); return; }
    send(Number(course), Number(speed));
  };
  const disabled = !canIssueOrders || !target || pending || uncertainOrder !== null;
  const receipt = receiptResource.data;
  const observedExecution = useRef<string | null>(null);
  useEffect(() => {
    if (receipt?.state === "executed" && observedExecution.current !== receipt.intent.intent_id) {
      observedExecution.current = receipt.intent.intent_id;
      onExecuted?.();
    }
  }, [receipt, onExecuted]);
  const receiptUnit = receipt ? projection.own_units.find((unit) => unit.id === receipt.intent.target) : null;

  return <section className="movement-orders" aria-label="Movement orders">
    <h2>Movement orders</h2>
    {units.length ? <>
      <form onSubmit={submit}>
        <fieldset disabled={pending || uncertainOrder !== null}>
          <label>Command unit<select value={target?.id ?? ""} onChange={(event) => setTargetId(event.target.value)}>
            {units.map((unit) => <option value={unit.id} key={unit.id}>{unit.name}</option>)}
          </select></label>
          <div className="movement-inputs">
            <label>Course (°)<input type="number" min="0" max="360" step="1" value={course} onChange={(event) => setCourse(event.target.value)} required /></label>
            <label>Speed (m/s)<input type="number" min="0" max="1000" step="1" value={speed} onChange={(event) => setSpeed(event.target.value)} required /></label>
          </div>
        </fieldset>
        <p className="movement-current">{target?.following_flight_path ? "Following scenario flight path" : target?.velocity ? `Current order: ${movementDescription(target.velocity)}` : "Current movement unavailable"}</p>
        <button className="command" type="submit" disabled={disabled} aria-busy={pending}>{pending ? "Sending order…" : "Send movement order"}</button>
        <div className="movement-shortcuts">
          <button className="secondary" type="button" disabled={disabled} onClick={() => { setCourse("0"); setSpeed("130"); send(0, 130); }}>Turn north</button>
          <button className="secondary" type="button" disabled={disabled} onClick={() => { setSpeed("0"); send(0, 0); }}>Stop unit</button>
        </div>
      </form>
      {uncertainOrder && <button className="command movement-retry" disabled={pending || !canRecoverOrder} onClick={() => void transmit(uncertainOrder)}>Retry order</button>}
      <div className={`movement-feedback ${receipt?.state ?? ""}`} role="status" aria-live="polite">
        {pending ? "Submitting movement order…" : feedback || (receipt ? <>
          <strong>{statusText[receipt.state]}{receipt.executed_tick !== undefined ? ` at tick ${receipt.executed_tick}` : ""}</strong>
          <span>{receiptUnit?.name ?? "Commanded unit"} · {movementDescription(receipt.intent.kind.Move)}</span>
          {receipt.error && <small>{receipt.error}</small>}
        </> : receiptId ? "Checking order status…" : "Choose a unit, course, and speed. Remote orders take effect after delivery." )}
        {receiptId && receiptResource.status === "reconnecting" && <small>Order status temporarily unavailable; reconnecting.</small>}
        {receiptId && receiptResource.status === "unavailable" && <small>{receiptResource.error?.message ?? "Order receipt unavailable."}</small>}
      </div>
    </> : <p className="muted">This role has no units under movement command.</p>}
  </section>;
}