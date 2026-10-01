import { useMemo } from "react";
import type { Role } from "./AuthorityWorkspace";
import type { Projection } from "./globeEntities";
import { formatBitRate } from "./networkModel";

const MAX_LINK_PREVIEW = 8;

export function OperationalInspector({ projection, role, onInspectNetwork }: {
  projection: Projection;
  role: Pick<Role, "location_unit_id" | "command_units">;
  onInspectNetwork: () => void;
}) {
  const communications = useMemo(() => {
    const names = new Map(projection.own_units.map((unit) => [unit.id, unit.name]));
    const relevant = new Set([role.location_unit_id, ...role.command_units]);
    const links = projection.communication_links.filter((link) => relevant.has(link.from_entity_id) || relevant.has(link.to_entity_id));
    links.sort((left, right) => Number(left.available) - Number(right.available)
      || Number((right.queued_packets ?? 0) > 0) - Number((left.queued_packets ?? 0) > 0)
      || Number(right.from_entity_id === role.location_unit_id) - Number(left.from_entity_id === role.location_unit_id)
      || left.id.localeCompare(right.id));
    return {
      names, preview: links.slice(0, MAX_LINK_PREVIEW), relevant: links.length,
      unavailable: projection.communication_links.filter((link) => !link.available).length,
      congested: projection.communication_links.filter((link) => (link.queued_packets ?? 0) > 0).length
    };
  }, [projection, role.location_unit_id, role.command_units]);

  return <aside className="inspector" aria-label="Operational picture">
    <h2>Operational picture</h2>
    <div className="metric"><span>Own units</span><strong>{projection.own_units.length}</strong></div>
    <div className="metric"><span>Tracks</span><strong>{projection.tracks.length}</strong></div>
    <h2>Communications</h2>
    <p className="communication-summary">{projection.communication_links.length.toLocaleString()} monitored links
      <span>{communications.unavailable.toLocaleString()} unavailable · {communications.congested.toLocaleString()} with queued traffic</span>
    </p>
    {communications.preview.length ? <>
      <p className="muted communication-preview-label">{communications.preview.length} of {communications.relevant.toLocaleString()} links involving your terminal or commanded units. Failures and queued traffic appear first.</p>
      {communications.preview.map((link) => <div className={`communication-status ${link.available ? "available" : "blocked"}`} key={link.id}>
        <span>{communications.names.get(link.from_entity_id) ?? "Unknown terminal"} → {communications.names.get(link.to_entity_id) ?? "Unknown terminal"}</span>
        <small>{link.available ? formatBitRate(link.effective_bit_rate_bps) : "Unavailable"}
          {link.jammed > 0 ? ` · ${Math.round(link.jammed * 100)}% interference` : ""}
          {(link.queued_packets ?? 0) > 0 ? ` · ${link.queued_packets} queued` : ""}</small>
      </div>)}
    </> : <p className="muted">No monitored links involving your role.</p>}
    {projection.communication_links.length > 0 && <button className="text-command" onClick={onInspectNetwork}>Inspect full network</button>}
    <h2>Tracks</h2>
    {projection.tracks.length ? projection.tracks.map((track) => <div className="track" key={track.track_id}>
      <span>Uncertain {track.target_side} contact</span><small>{Math.round(track.identity_confidence * 100)}% identity</small>
    </div>) : <p className="muted">No reports received.</p>}
  </aside>;
}