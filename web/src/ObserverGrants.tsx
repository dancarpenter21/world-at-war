import { useState } from "react";
import { apiRequest } from "./apiClient";
import { usePollingResource } from "./usePollingResource";
import type { ObserverRole } from "./observerTypes";
type Participant = { player_id: string; display_name: string; observer_seats: string[] };
export function ObserverGrants({ apiBase, gameId, seats, onChanged, onClose }: {
  apiBase: string; gameId: string; seats: ObserverRole[]; onChanged: () => void; onClose: () => void;
}) {
  const people = usePollingResource<Participant[]>(gameId, signal => apiRequest(apiBase, `/v1/games/${gameId}/participants`, { signal }), 2000);
  const [error, setError] = useState("");
  const [pending, setPending] = useState(false);
  async function assign(seat: string, player: string) {
    setPending(true); setError("");
    try {
      await apiRequest(apiBase, `/v1/games/${gameId}/roles/${seat}/observer-grant`, {
        method: player ? "PUT" : "DELETE", body: JSON.stringify(player ? { target_player_id: player } : {})
      });
      people.refresh(); onChanged();
    } catch (cause) { setError((cause as Error).message); }
    finally { setPending(false); }
  }
  return <div className="lobby-stage observer-grants"><section className="scenario-modal" role="dialog" aria-modal="true" aria-label="Observer access">
    <div className="modal-body"><h2>Observer access</h2><p>Grant a joined guest permission to claim a read-only ground-truth seat. Replacing or revoking access releases the current lease.</p>
      {(error || people.error) && <p role="alert">{error || people.error?.message}</p>}
      {seats.map(seat => <label key={seat.id}>{seat.name}<select aria-label={`${seat.name} access`} disabled={pending || !people.data}
        value={people.data?.find(p => p.observer_seats.includes(seat.id))?.player_id ?? ""}
        onChange={event => void assign(seat.id, event.target.value)}>
        <option value="">No access</option>
        {people.data?.map(p => <option key={p.player_id} value={p.player_id}>{p.display_name} · {p.player_id.slice(-6)}</option>)}
      </select></label>)}
      <button className="secondary" onClick={onClose}>Close observer access</button>
    </div>
  </section></div>;
}
