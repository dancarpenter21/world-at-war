import type { CombatProjection } from "./globeEntities";
import type { PollingStatus } from "./usePollingResource";

export type Game = {
  id: string; title: string; status: "lobby" | "running" | "paused";
  host_player_id: string; player_roles_available: number; space_catalog_enabled: boolean;
  operational_error?: string | null;
  mission_complete?: boolean;
  settling_reports?: boolean;
};

export function GameSessionControls({ game, isHost, pending, onControl }: {
  game: Game; isHost: boolean; pending: "running" | "paused" | null;
  onControl: (status: "running" | "paused") => void;
}) {
  if (game.status === "lobby") return null;
  return <div className="game-session-controls">
    <span className={"game-state " + game.status}>{game.settling_reports ? "Radio reporting" : game.status === "paused" ? "Paused" : "Running"}</span>
    {isHost && <button className="secondary" disabled={pending !== null || game.mission_complete} title={game.mission_complete ? "This exercise has ended. Create a new game to retry." : undefined} aria-busy={pending !== null}
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