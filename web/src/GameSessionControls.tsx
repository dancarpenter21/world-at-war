import type { CombatProjection } from "./globeEntities";
import type { PollingStatus } from "./usePollingResource";

export type Game = {
  id: string; title: string; status: "lobby" | "running" | "paused";
  control_revision?: number; blue_roles_available?: number; red_roles_available?: number; red_control?: "human" | "ai" | null;
  host_player_id: string; player_roles_available: number; space_catalog_enabled: boolean;
  operational_error?: string | null;
  mission_complete?: boolean;
  settling_reports?: boolean;
  reporting_pending?: boolean;
  time_scale?: number; simulation_tick_hz?: number; frame?: number; tick?: number; radio_tick?: number; elapsed_seconds?: number;
};

export function GameSessionControls({ game, canControl, pending, onControl, onTimeScale }: {
  game: Game; canControl: boolean; pending: "running" | "paused" | "speed" | null;
  onControl: (status: "running" | "paused") => void;
  onTimeScale: (scale: number) => void;
}) {
  if (game.status === "lobby") return null;
  return <div className="game-session-controls">
    <span className={"game-state " + game.status}>{game.settling_reports ? "Radio reporting" : game.status === "paused" ? "Paused" : "Running"}</span>
    <span className="simulation-time">Tick {game.tick ?? 0}{game.settling_reports ? ` · radio ${game.radio_tick ?? game.tick ?? 0}` : ""} · {formatElapsed(game.elapsed_seconds ?? 0)}</span>
    <span className="clock-rate" title="Simulation frame rate; speed scales game time per frame">{game.simulation_tick_hz ?? 60} Hz</span>
    {canControl ? <label className="time-scale">Speed <select aria-label="Simulation speed" value={game.time_scale ?? 1} disabled={pending !== null} onChange={event => onTimeScale(Number(event.target.value))}>{[0.25, 1, 2, 5, 10, 30, 60].map(scale => <option key={scale} value={scale}>{scale}×</option>)}</select></label> : <span aria-label="Simulation speed">{game.time_scale ?? 1}×</span>}
    {canControl && <button className="secondary" disabled={pending !== null || (game.mission_complete && !game.reporting_pending)} title={game.mission_complete && !game.reporting_pending ? "This exercise has ended. Create a new game to retry." : undefined} aria-busy={pending !== null}
      onClick={() => onControl(game.status === "running" ? "paused" : "running")}>
      {pending === "paused" ? "Pausing…" : pending === "running" ? "Resuming…" : game.status === "running" ? "Pause scenario" : "Resume scenario"}
    </button>}
  </div>;
}

export function GameSessionNotice({ game, status, error, controlError, hasProjection, onRetry, mission }: {
  game: Game; status: PollingStatus; error: Error | null; controlError: string;
  hasProjection: boolean; onRetry: () => void; mission?: CombatProjection["mission"];
}) {
  const interrupted = status === "reconnecting" || status === "unavailable";
  const missionFinished = mission && mission.status !== "active";
  if (!missionFinished && !interrupted && game.status !== "paused" && !game.operational_error && !controlError && hasProjection) return null;
  return <div className={"game-session-notice " + (interrupted || game.operational_error || controlError ? "warning" : "")} role="status" aria-live="polite">
    <div>
      <strong>{game.operational_error ? "Scenario paused by the server" : interrupted ? "Operational picture interrupted" : missionFinished ? (mission.status === "succeeded" ? "Mission complete" : "Mission failed") : game.status === "paused" ? "Scenario paused" : hasProjection ? "Scenario control failed" : "Loading operational picture"}</strong>
      <span>{game.operational_error ?? (interrupted
        ? (hasProjection ? "Showing the last received map. Orders are unavailable until the connection recovers." : "Waiting for the server. Retrying the connection.")
        : missionFinished ? `${mission.reason} at tick ${mission.finished_tick}. ${game.settling_reports ? "Final radio reports are still settling." : "The exercise has ended; create a new game to retry."}` : game.status === "paused" ? "Simulation time is stopped. The map and inspectors remain available." : hasProjection ? controlError : "Waiting for your role's current map.")}</span>
      {interrupted && error && <small>{error.message}</small>}
      {controlError && (interrupted || game.status === "paused" || game.operational_error) && <small>{controlError}</small>}
    </div>
    {interrupted && <button className="secondary" onClick={onRetry}>Retry connection</button>}
  </div>;
}
export function formatElapsed(seconds: number): string {
  const value = Math.max(0, Math.floor(seconds));
  return `${Math.floor(value / 3600).toString().padStart(2, "0")}:${Math.floor(value / 60 % 60).toString().padStart(2, "0")}:${(value % 60).toString().padStart(2, "0")}`;
}
