import { authenticatedFetch } from "./apiClient";
import { useEffect, useState } from "react";

export type ClientDiagnostics = {
  lastReceivedAt: number | null;
  requestMs: number | null;
  mapUpdateMs: number | null;
  failed: boolean;
};
type Summary = { samples: number; last: number | null; p95: number | null; max: number | null };
type Diagnostics = {
  tick: number; status: "lobby" | "running" | "paused"; window_capacity: number;
  target_frame_hz: number; committed_frame_hz: number | null; committed_frame_interval_ms: Summary;
  requested_time_scale: number; achieved_time_scale: number | null; committed_frame_overruns: number;
  last_committed_frame_age_ms: number | null;
  server_tick_ms: Summary; last_tick_age_ms: number | null; server_tick_overruns: number;
  projection: { tick: number; build_ms: Summary; serialization_ms: Summary; snapshot_bytes: number; visible_queue_packets: number; visible_queue_bytes: number } | null;
};
const ms = (value: number | null | undefined) => value == null ? "Waiting for sample" : `${value.toFixed(1)} ms`;

export function connectionHealth(client: ClientDiagnostics, now: number): "Waiting" | "Reconnecting" | "Stale" | "Connected" {
  if (client.failed) return "Reconnecting";
  if (client.lastReceivedAt === null) return "Waiting";
  return now - client.lastReceivedAt > 5000 ? "Stale" : "Connected";
}

export function DiagnosticsPanel({ apiBase, gameId, playerId, roleId, client, onClose }: {
  apiBase: string; gameId: string; playerId: string; roleId: string; client: ClientDiagnostics; onClose: () => void;
}) {
  const [data, setData] = useState<Diagnostics | null>(null);
  const [error, setError] = useState("");
  const [now, setNow] = useState(performance.now());
  useEffect(() => {
    let active = true;
    let timer: number | undefined;
    let controller: AbortController | undefined;
    setData(null);
    const poll = async () => {
      controller = new AbortController();
      const timeout = window.setTimeout(() => controller?.abort(), 8000);
      try {
        const response = await authenticatedFetch(`${apiBase}/v1/games/${gameId}/diagnostics?player_id=${playerId}&role_id=${roleId}`, { credentials: "include", signal: controller.signal });
        if (!response.ok) throw new Error("Diagnostics unavailable");
        const next = await response.json() as Diagnostics;
        if (active) { setData(next); setError(""); }
      } catch {
        if (active) setError("Diagnostics unavailable; retrying. Last measurements may be stale.");
      } finally {
        window.clearTimeout(timeout);
        if (active) timer = window.setTimeout(() => void poll(), 2000);
      }
    };
    void poll();
    const clock = window.setInterval(() => setNow(performance.now()), 500);
    return () => { active = false; controller?.abort(); window.clearTimeout(timer); window.clearInterval(clock); };
  }, [apiBase, gameId, playerId, roleId]);
  const health = connectionHealth(client, now);
  return <section className="diagnostics-panel" aria-label="Simulation diagnostics">
    <header><h2>Diagnostics</h2><button onClick={onClose}>Close diagnostics</button></header>
    <p role="status">Connection: {health}</p>
    {error && <p role="alert">{error}</p>}
    <dl>
      <dt>Simulation</dt><dd>{data ? `${data.status} · tick ${data.tick}` : "Waiting for sample"}</dd>
      <dt>Last map update received</dt><dd>{client.lastReceivedAt === null ? "Waiting for sample" : `${Math.max(0, now - client.lastReceivedAt).toFixed(0)} ms ago`}</dd>
      <dt>State request</dt><dd>{ms(client.requestMs)}</dd>
      <dt>Map update processing</dt><dd>{ms(client.mapUpdateMs)}</dd>
      <dt>Committed frame rate / target</dt><dd>{data?.committed_frame_hz == null ? "Waiting for sample" : `${data.committed_frame_hz.toFixed(1)} Hz`} / {data?.target_frame_hz ?? 60} Hz</dd>
      <dt>Game speed, requested / achieved</dt><dd>{data ? `${data.requested_time_scale}×` : "Waiting for sample"} / {data?.achieved_time_scale == null ? "Waiting for sample" : `${data.achieved_time_scale.toFixed(2)}×`}</dd>
      <dt>Committed frame interval, latest / p95</dt><dd>{ms(data?.committed_frame_interval_ms?.last)} / {ms(data?.committed_frame_interval_ms?.p95)}</dd>
      <dt>Intervals over frame period</dt><dd>{data ? `${data.committed_frame_overruns ?? 0} / ${data.committed_frame_interval_ms?.samples ?? 0}` : "Waiting for sample"}</dd>
      <dt>Last committed frame</dt><dd>{ms(data?.last_committed_frame_age_ms)}</dd>
      <dt>Frame processing, latest / p95</dt><dd>{ms(data?.server_tick_ms.last)} / {ms(data?.server_tick_ms.p95)}</dd>
      <dt>Frames over one second of processing</dt><dd>{data ? `${data.server_tick_overruns} / ${data.server_tick_ms.samples}` : "Waiting for sample"}</dd>
      <dt>Role projection, latest / p95</dt><dd>{ms(data?.projection?.build_ms.last)} / {ms(data?.projection?.build_ms.p95)}</dd>
      <dt>Snapshot size</dt><dd>{data?.projection ? `${(data.projection.snapshot_bytes / 1024).toFixed(1)} KiB (uncompressed)` : "Waiting for sample"}</dd>
      <dt>Visible link queues</dt><dd>{data?.projection ? `${data.projection.visible_queue_packets} packets · ${(data.projection.visible_queue_bytes / 1024).toFixed(1)} KiB` : "Waiting for sample"}</dd>
    </dl>
    {data?.status === "paused" && <p>Simulation is paused; a stationary tick is expected.</p>}
    {data?.status === "running" && data.last_committed_frame_age_ms !== null && data.last_committed_frame_age_ms > 3000 && <p>Simulation progress is delayed.</p>}
    <small>Committed rates include persistence and scheduling delays. Frame processing excludes persistence. Server timings cover up to {data?.window_capacity ?? 120} recent samples. Queue totals use only this role’s visible links. Map processing excludes GPU rendering.</small>
  </section>;
}
