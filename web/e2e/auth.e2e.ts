import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { backendUrl, startBackend, type Backend } from "./support/server";

let backend: Backend;
let extra: BrowserContext[];
test.beforeEach(async () => { extra = []; backend = await startBackend(); });
test.afterEach(async ({}, info) => {
  if (info.status !== info.expectedStatus) await info.attach("backend.log", { body: backend.output(), contentType: "text/plain" });
  await Promise.all(extra.map(context => context.close())); await backend.stop();
});
async function open(page: Page) {
  await page.route("https://tile.openstreetmap.org/**", route => route.abort());
  await page.goto("/");
  await expect(page.getByRole("button", { name: /^Command Link Exercise/ })).toBeVisible();
}
async function create(page: Page) {
  await open(page);
  await page.getByRole("button", { name: /^Command Link Exercise/ }).click();
  await page.getByLabel("Game title").fill("Session ownership exercise");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  await page.getByRole("button", { name: /^Exercise Commander/ }).click();
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.locator(".globe canvas")).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
}
async function selection(page: Page) {
  return page.evaluate(() => JSON.parse(localStorage.getItem("world-at-war-session")!) as { player_id: string; game_id: string; role_id: string; lease_generation: number });
}
async function headers(page: Page) {
  const session = await (await page.context().request.get(backendUrl + "/v1/auth/session")).json();
  return { "x-csrf-token": session.csrf_token as string, origin: new URL(page.url()).origin };
}
async function observeStream(page: Page, network: boolean) {
  const selected = await selection(page);
  await page.evaluate(({ base, selected, network }) => {
    const url = new URL(`${base}/v1/games/${selected.game_id}/${network ? "network/stream" : "stream"}`);
    url.protocol = "ws:"; url.search = new URLSearchParams({ role_id: selected.role_id, lease_generation: String(selected.lease_generation) }).toString();
    const socket = new WebSocket(url);
    const observation = { frames: 0, closeCode: 0 };
    (window as any).authStream = observation;
    socket.onmessage = () => observation.frames++;
    socket.onclose = event => { observation.closeCode = event.code; };
  }, { base: backendUrl, selected, network });
  await expect.poll(() => page.evaluate(() => (window as any).authStream.frames)).toBeGreaterThan(0);
}

test("independent guests cannot assert identity, bypass CSRF or take another player's role", async ({ page, browser }) => {
  await create(page);
  const owned = await selection(page);
  const cookie = (await page.context().cookies(backendUrl + "/v1")).find(cookie => cookie.name === "world_at_war_session")!;
  expect(cookie.httpOnly).toBe(true); expect(cookie.sameSite).toBe("Strict"); expect(cookie.value).not.toContain(owned.player_id);
  const other = await browser.newContext({ baseURL: page.url() }); extra.push(other);
  const outsider = await other.newPage(); await open(outsider);
  const otherHeaders = await headers(outsider);
  const prefix = `${backendUrl}/v1/games/${owned.game_id}`;
  const otherIdentity = await (await other.request.get(backendUrl + "/v1/auth/session")).json();
  expect(otherIdentity.player_id).not.toBe(owned.player_id);
  expect((await other.request.post(prefix + "/pause", { headers: otherHeaders, data: {} })).status()).toBe(403);
  expect((await other.request.post(prefix + "/pause", { headers: otherHeaders, data: { player_id: owned.player_id } })).status()).toBe(400);
  for (const endpoint of ["state", "network", "network/events", "planning", "diagnostics"]) {
    expect((await other.request.get(`${prefix}/${endpoint}?role_id=${owned.role_id}&lease_generation=${owned.lease_generation}`)).status()).toBe(403);
  }
  expect((await other.request.post(prefix + "/join", { headers: otherHeaders, data: { display_name: "Other" } })).ok()).toBe(true);
  expect((await other.request.post(`${prefix}/roles/${owned.role_id}/claim`, { headers: otherHeaders, data: {} })).status()).toBe(409);
  expect((await page.context().request.post(prefix + "/start", { data: {} })).status()).toBe(403);
  expect((await page.context().request.post(prefix + "/start", { headers: { ...await headers(page), origin: "https://untrusted.example" }, data: {} })).status()).toBe(403);
  expect((await page.context().request.get(`${prefix}/state?role_id=${owned.role_id}&lease_generation=${owned.lease_generation + 1}`)).status()).toBe(403);
  await page.reload();
  await expect(page.locator(".globe canvas")).toBeVisible({ timeout: 20_000 });
  expect((await selection(page)).lease_generation).toBe(owned.lease_generation);
});

test("tabs share a lease and release revokes a paused stream before another guest claims it", async ({ page, browser }) => {
  await create(page); const owned = await selection(page);
  const tab = await page.context().newPage(); await tab.goto(page.url());
  await expect(tab.locator(".globe canvas")).toBeVisible({ timeout: 20_000 });
  expect(await selection(tab)).toEqual(owned);
  await observeStream(tab, true);
  const other = await browser.newContext({ baseURL: page.url() }); extra.push(other);
  const outsider = await other.newPage(); await open(outsider);
  await outsider.getByRole("button", { name: "Join game", exact: true }).click();
  await outsider.getByRole("button", { name: /Session ownership exercise.*paused/ }).click();
  await expect(outsider.getByRole("button", { name: /^Exercise Commander/ })).toBeDisabled();
  await page.getByRole("button", { name: "Release role", exact: true }).click();
  await expect.poll(() => tab.evaluate(() => (window as any).authStream.closeCode)).toBe(1008);
  await expect(tab.locator(".globe")).toHaveCount(0);
  await expect(outsider.getByRole("button", { name: /^Exercise Commander/ })).toBeEnabled();
  await outsider.getByRole("button", { name: /^Exercise Commander/ }).click();
  await expect(outsider.locator(".globe canvas")).toBeVisible({ timeout: 20_000 });
  expect((await selection(outsider)).lease_generation).toBeGreaterThan(owned.lease_generation);
  expect((await page.context().request.post(`${backendUrl}/v1/games/${owned.game_id}/roles/${owned.role_id}/resume`, { headers: await headers(page), data: {} })).status()).toBe(403);
  await outsider.screenshot({ path: test.info().outputPath("role-reassigned.png") });
});

test("logout closes a paused state stream and server restart discards guest identity and selection", async ({ page }) => {
  await create(page); const before = await selection(page);
  await observeStream(page, false);
  const oldCookie = (await page.context().cookies(backendUrl + "/v1")).find(cookie => cookie.name === "world_at_war_session")!;
  expect((await page.context().request.post(backendUrl + "/v1/auth/logout", { headers: await headers(page), data: {} })).ok()).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).authStream.closeCode)).toBe(1008);
  await expect(page.locator(".globe")).toHaveCount(0);
  expect((await page.context().request.get(backendUrl + "/v1/auth/session", { headers: { cookie: `world_at_war_session=${oldCookie.value}` } })).status()).toBe(401);
  await expect(page.getByRole("button", { name: /^Command Link Exercise/ })).toBeVisible();
  await create(page);
  const second = await selection(page); expect(second.player_id).not.toBe(before.player_id);
  await backend.stop(); backend = await startBackend();
  await page.reload();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect(page.locator(".globe")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  const current = await (await page.context().request.get(backendUrl + "/v1/auth/session")).json();
  expect(current.player_id).not.toBe(second.player_id);
});

test("ending a guest session clears identity and operational data in every shared tab", async ({ page }) => {
  await create(page); const before = await selection(page);
  const tab = await page.context().newPage(); await tab.goto(page.url());
  await expect(tab.locator(".globe canvas")).toBeVisible({ timeout: 20_000 });
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await page.getByRole("button", { name: "End guest session", exact: true }).click();
  await expect(tab.locator(".globe")).toHaveCount(0);
  await expect(tab.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect(tab.getByRole("button", { name: "End guest session", exact: true })).toBeVisible();
  const current = await (await tab.context().request.get(backendUrl + "/v1/auth/session")).json();
  expect(current.player_id).not.toBe(before.player_id);
  await expect.poll(() => tab.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  await expect(tab.getByRole("button", { name: "Pause scenario", exact: true })).toHaveCount(0);
});
