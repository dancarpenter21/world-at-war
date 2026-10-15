import { useState } from "react";
import { apiRequest } from "./apiClient";
import { AgentControllers } from "./AgentControllers";

export type Team = "blue" | "red" | "white_cell";
export type TeamView = {
  team: Team | null; referee_seat_id: string | null; can_manage: boolean; can_control: boolean;
  control_revision: number; red_available: boolean;
  requests: { id: string; player_id: string; team: Team; status: "pending" | "approved" | "declined" | "cancelled" }[];
  members: { player_id: string; display_name: string; team: Team | null; referee_seat_id: string | null; active: boolean }[];
};
export const teamName = (team: Team | null | undefined) => team === "white_cell" ? "White Cell" : team === "red" ? "Red" : team === "blue" ? "Blue" : "Choose a team";
export function TeamPanel({ apiBase, gameId, playerId, hostId, view, onChanged, onEnter, onClose }: {
  apiBase: string; gameId: string; playerId: string; hostId: string; view: TeamView | null;
  onChanged: () => void; onEnter: (id: string) => void; onClose?: () => void;
}) {
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  async function mutate(path: string, body: unknown, method = "POST") {
    setPending(true); setError("");
    try { await apiRequest(apiBase, `/v1/games/${gameId}/teams${path}`, { method, body: JSON.stringify(body) }); onChanged(); }
    catch (cause) { setError((cause as Error).message); }
    finally { setPending(false); }
  }
  function request(team: Team) {
    if (view?.team === "white_cell" && team !== "white_cell" && !window.confirm("You have seen hidden exercise information. Request a transfer to a combatant team?")) return;
    void mutate("/requests", { request_id: crypto.randomUUID(), team });
  }
  const ownPending = view?.requests.find(r => r.player_id === playerId && r.status === "pending");
  return <section className="team-panel" aria-label="Team and players">
    <div className="team-panel-heading"><h2>Team & players</h2>{onClose && <button className="secondary" onClick={onClose}>Close team panel</button>}</div>
    <p>Current team: <strong>{teamName(view?.team)}</strong>. White Cell referees see all sides and require approval.</p>
    {error && <p role="alert">{error}</p>}
    {!view ? <p role="status">Loading teams…</p> : <>
      <div className="team-cards">{(["blue", "red", "white_cell"] as Team[]).map(team => <button className={`team-card ${view.team === team ? "selected" : ""}`} key={team}
        disabled={pending || Boolean(ownPending) || view.team === team || (team === "red" && !view.red_available)} onClick={() => request(team)}>
        <strong>{teamName(team)}</strong><span>{team === "red" && !view.red_available ? "AI controlled or unavailable" : view.team === team ? "Your team" : view.team ? "Request transfer" : team === "white_cell" ? "Request White Cell" : `Join ${teamName(team)}`}</span>
      </button>)}</div>
      {ownPending && <p role="status">Waiting for approval to join {teamName(ownPending.team)}. <button className="secondary" disabled={pending} onClick={() => void mutate(`/requests/${ownPending.id}/cancel`, {})}>Cancel request</button></p>}
      {view.requests.filter(r => r.player_id === playerId && r.status !== "pending").slice(-3).map(r => <p key={r.id} role="status">{teamName(r.team)} request: {r.status}</p>)}
      {view.team === "white_cell" && view.referee_seat_id && <button className="command" onClick={() => onEnter(view.referee_seat_id!)}>Enter White Cell</button>}
      {view.can_manage && <><h3>Join and transfer requests</h3>{view.requests.filter(r => r.status === "pending").length === 0 && <p>No pending requests.</p>}
        {view.requests.filter(r => r.status === "pending").map(r => <div className="team-request" key={r.id}><span>{view.members.find(m => m.player_id === r.player_id)?.display_name ?? "Player"} → {teamName(r.team)}</span>
          <button className="command" disabled={pending || (r.player_id === playerId && hostId !== playerId)} onClick={() => void mutate(`/requests/${r.id}/decision`, { approve: true })}>Approve</button>
          <button className="secondary" disabled={pending || (r.player_id === playerId && hostId !== playerId)} onClick={() => void mutate(`/requests/${r.id}/decision`, { approve: false })}>Decline</button></div>)}
        <h3>Players</h3><ul>{view.members.map(m => <li key={m.player_id}>{m.display_name} · {teamName(m.team)}{m.team === "white_cell" ? m.active ? " · connected referee" : " · offline referee" : ""}
          {hostId === playerId && m.player_id !== playerId && m.team && <button className="secondary" disabled={pending} onClick={() => void mutate(`/members/${m.player_id}`, {}, "DELETE")}>Remove from team</button>}</li>)}</ul></>}
      {view.can_manage && <AgentControllers apiBase={apiBase} gameId={gameId} onChanged={onChanged} />}
      {view.team && <button className="secondary" disabled={pending} onClick={() => void mutate(`/members/${playerId}`, {}, "DELETE")}>Leave team</button>}
    </>}
  </section>;
}
