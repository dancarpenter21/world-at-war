import { request as createRequest, type APIRequestContext, type Page } from "@playwright/test";

/** Test labels select separate real cookie jars; labels are never sent as identity. */
export function playerRequests(page: Page, primaryLabel: string) {
  const clients = new Map<string, APIRequestContext>([[primaryLabel, page.context().request]]);
  const owned: APIRequestContext[] = [];
  async function send(input: string, method: "GET" | "POST", options?: { data?: unknown }) {
    const url = new URL(input);
    const origin = page.url().startsWith("http") ? new URL(page.url()).origin : `http://${process.env.E2E_BROWSER_HOST ?? "127.0.0.1"}:4173`;
    const data = options?.data && typeof options.data === "object" ? { ...options.data } as Record<string, unknown> : options?.data;
    const label = String((data as Record<string, unknown> | undefined)?.player_id ?? url.searchParams.get("player_id") ?? primaryLabel);
    let client = clients.get(label);
    if (!client) { client = await createRequest.newContext(); clients.set(label, client); owned.push(client); }
    if (method === "GET" && !url.pathname.startsWith("/v1/games")) return client.get(input);
    let sessionResponse = await client.get(url.origin + "/v1/auth/session");
    if (sessionResponse.status() === 401) sessionResponse = await client.post(url.origin + "/v1/auth/guest", {
      headers: { origin }, data: { display_name: "Test player" }
    });
    const session = await sessionResponse.json();
    const headers = { "x-csrf-token": session.csrf_token ?? "", origin };
    url.searchParams.delete("player_id"); url.searchParams.delete("host_player_id");
    if (data && typeof data === "object") { delete (data as Record<string, unknown>).player_id; delete (data as Record<string, unknown>).host_player_id; }
    const prefix = url.pathname.match(/^\/v1\/games\/[^/]+/)?.[0];
    const roleId = url.searchParams.get("role_id");
    if (prefix && roleId && !url.searchParams.has("lease_generation")) {
      const roles = await (await client.get(url.origin + prefix + "/roles")).json();
      url.searchParams.set("lease_generation", String(roles.find((role: { id: string }) => role.id === roleId)?.lease_generation ?? 0));
    }
    if (prefix && method === "POST" && url.pathname.endsWith("/claim")) {
      await client.post(url.origin + prefix + "/join", { headers, data: { display_name: "Test player" } });
    }
    return client.fetch(url.toString(), { method, headers, ...(data === undefined ? {} : { data }) });
  }
  return {
    get: (url: string) => send(url, "GET"),
    post: (url: string, options?: { data?: unknown }) => send(url, "POST", options),
    register: (label: string, client: APIRequestContext) => { clients.set(label, client); },
    dispose: () => Promise.all(owned.map(client => client.dispose()))
  };
}
type StreamFrame = { projection: { messages: { state: string; message: { id: string } }[] } };
export async function browserStream(page: Page, input: string) {
  const id = `stream-${Math.random()}`;
  await page.evaluate(async ({ input, id }) => {
    const url = new URL(input);
    url.searchParams.delete("player_id");
    const http = new URL(url); http.protocol = http.protocol === "wss:" ? "https:" : "http:";
    const prefix = url.pathname.match(/^\/v1\/games\/[^/]+/)![0];
    const roles = await (await fetch(http.origin + prefix + "/roles", { credentials: "include" })).json();
    url.searchParams.set("lease_generation", String(roles.find((r: { id: string }) => r.id === url.searchParams.get("role_id")).lease_generation));
    const socket = new WebSocket(url); const frames: unknown[] = [];
    (window as any)[id] = { socket, frames };
    socket.onmessage = event => frames.push(JSON.parse(String(event.data)));
  }, { input, id });
  return {
    frames: () => page.evaluate(id => (window as any)[id].frames as StreamFrame[], id),
    close: () => page.evaluate(id => { (window as any)[id].socket.close(); delete (window as any)[id]; }, id)
  };
}
