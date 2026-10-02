export type SavedSession = {
  player_id: string;
  game_id: string;
  role_id: string;
  lease_generation: number;
};

/** A saved selection is only a hint; server lease and projection checks still authorize it. */
export function parseSavedSession(raw: string | null): SavedSession | null {
  if (!raw) return null;
  let parsed: unknown;
  try { parsed = JSON.parse(raw); } catch { return null; }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
  const value = parsed as Record<string, unknown>;
  if (![value.player_id, value.game_id, value.role_id].every((item) => typeof item === "string" && item.length > 0)
    || typeof value.lease_generation !== "number" || !Number.isSafeInteger(value.lease_generation) || value.lease_generation < 0) return null;
  return {
    player_id: value.player_id as string,
    game_id: value.game_id as string,
    role_id: value.role_id as string,
    lease_generation: value.lease_generation
  };
}
