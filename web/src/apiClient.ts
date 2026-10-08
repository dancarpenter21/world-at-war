export class ApiError extends Error {
  constructor(message: string, readonly status: number, readonly code?: string) {
    super(message); this.name = "ApiError";
  }
}
export type GuestSession = { player_id: string; display_name: string; csrf_token: string; expires_unix: number };
let guest: GuestSession | undefined;
let bootstrap: Promise<GuestSession> | undefined;
let authRevision = 0;
export const GUEST_REVOKED_KEY = "world-at-war-guest-revoked";
const leases = new Map<string, number>();
export function rememberLease(game: string, role: string, generation: number) { leases.set(`${game}/${role}`, generation); }
export function leaseGeneration(game: string, role: string) { return leases.get(`${game}/${role}`); }
export function forgetLease(game: string, role: string) { leases.delete(`${game}/${role}`); }
export function clearGuest(notifyOtherTabs = false) {
  const previous = guest;
  guest = undefined; bootstrap = undefined; leases.clear(); authRevision++;
  if (notifyOtherTabs && previous) localStorage.setItem(GUEST_REVOKED_KEY, crypto.randomUUID());
}
async function decode<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const body: unknown = await response.json().catch(() => null);
    const detail = body && typeof body === "object" ? body as Record<string, unknown> : {};
    throw new ApiError(typeof detail.error === "string" ? detail.error : response.statusText || `Request failed (HTTP ${response.status}).`, response.status, typeof detail.code === "string" ? detail.code : undefined);
  }
  return response.json() as Promise<T>;
}
export function ensureGuest(apiBase: string): Promise<GuestSession> {
  if (guest) return Promise.resolve(guest);
  if (bootstrap) return bootstrap;
  const revision = authRevision;
  const connect = async () => {
    let response = await fetch(apiBase + "/v1/auth/session", { credentials: "include" });
    if (response.status === 401) response = await fetch(apiBase + "/v1/auth/guest", {
      method: "POST", credentials: "include", headers: { "content-type": "application/json" }, body: JSON.stringify({ display_name: "Commander" })
    });
    const result = await decode<GuestSession>(response);
    if (revision !== authRevision) throw new ApiError("Guest session changed while connecting.", 401);
    guest = result;
    return result;
  };
  const pending = Promise.resolve(navigator.locks ? navigator.locks.request("world-at-war-guest", connect) : connect()).finally(() => { if (revision === authRevision) bootstrap = undefined; });
  bootstrap = pending;
  return pending;
}

/** All game requests derive identity from the cookie; local IDs are presentation only. */
export function authorizedUrl(input: string): URL {
  const url = new URL(input, window.location.href);
  url.searchParams.delete("player_id"); url.searchParams.delete("host_player_id");
  const game = url.pathname.match(/\/v1\/games\/([^/]+)/)?.[1];
  const role = url.searchParams.get("role_id");
  if (game && role && !url.searchParams.has("lease_generation")) {
    url.searchParams.set("lease_generation", String(leaseGeneration(game, role) ?? 0));
  }
  return url;
}
export async function authenticatedFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = authorizedUrl(input);
  const headers = new Headers(init?.headers);
  headers.set("content-type", "application/json");
  const mutation = !["GET", "HEAD", "OPTIONS"].includes((init?.method ?? "GET").toUpperCase());
  if (mutation && url.pathname.startsWith("/v1/")) {
    const session = await ensureGuest(url.origin);
    headers.set("x-csrf-token", session.csrf_token);
  }
  let body = init?.body;
  if (typeof body === "string" && url.pathname.startsWith("/v1/games")) {
    const value = JSON.parse(body) as Record<string, unknown>;
    delete value.player_id; delete value.host_player_id;
    body = JSON.stringify(value);
  }
  const response = await fetch(url, { ...init, body, credentials: "include", headers });
  if (response.status === 401 && url.pathname.startsWith("/v1/games")) {
    clearGuest(true); window.dispatchEvent(new Event("guest-session-lost"));
  }
  return response;
}
export async function apiRequest<T>(apiBase: string, path: string, init?: RequestInit): Promise<T> {
  return decode<T>(await authenticatedFetch(apiBase + path, init));
}
