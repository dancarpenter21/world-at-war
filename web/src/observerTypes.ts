import type { CommunicationLink, Position, Unit } from "./globeEntities";
export type ObserverRole = {
  id: string; name: string; kind: "controller" | "monitor"; observer_kind: "controller" | "monitor";
  held: boolean; held_by_you: boolean; claimable: boolean; ai_controlled: false;
  lease_generation: number; lease_state?: "active" | "reserved" | "available";
};
export type TruthUnit = Unit & { side: string; fuel: number; ammunition: number; destroyed: boolean };
export type TruthWeapon = {
  id: string; kind: string; side: string | null; launcher_id: string | null;
  position: Position | null; aim_point: Position | null; impact_tick: number | null;
  phase: string; provider_id: string | null;
};
export type TruthProjection = {
  tick: number; units: TruthUnit[]; weapons: TruthWeapon[];
  jamming_regions: { id: string; name: string; center: Position; radius_m: number }[];
  communication_links: CommunicationLink[];
};
