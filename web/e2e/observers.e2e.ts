import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { backendUrl, startBackend, type Backend } from "./support/server";

let backend: Backend;
let extra: BrowserContext[];
test.beforeEach(async () => { extra = []; backend = await startBackend(); });
test.afterEach(async ({}, info) => {
  if (info.status !== info.expectedStatus && backend) await info.attach("backend.log", { body: backend.output(), contentType: "text/plain" });
  await Promise.all(extra.map(c => c.close())); await backend?.stop();
});
async function open(page: Page) {
  await page.context().route("https://tile.openstreetmap.org/**", route => route.abort());
  await page.goto("/");
  await expect(page.getByRole("button", { name: /^Regional Joint Campaign/ })).toBeVisible();
}
async function headers(page: Page) {
  const session = await (await page.request.get(backendUrl + "/v1/auth/session")).json();
  return { "x-csrf-token": session.csrf_token as string, origin: new URL(page.url()).origin };
}
async function selection(page: Page) {
  return page.evaluate(() => JSON.parse(localStorage.getItem("world-at-war-session")!) as { game_id: string; role_id: string; lease_generation: number });
}
async function stream(page: Page) {
  const seat = await selection(page);
  await page.evaluate(({ seat, base }) => {
    const url = new URL(`${base}/v1/games/${seat.game_id}/truth/stream`);
    url.protocol = "ws:"; url.search = new URLSearchParams({ role_id: seat.role_id, lease_generation: String(seat.lease_generation) }).toString();
    const socket = new WebSocket(url);
    const result = { frames: 0, close: 0 };
    (window as any).truthStream = result;
    socket.onmessage = () => result.frames++;
    socket.onclose = event => { result.close = event.code; };
  }, { seat, base: backendUrl });
  await expect.poll(() => page.evaluate(() => (window as any).truthStream.frames)).toBeGreaterThan(1);
}

test("host grants read-only truth; reload, reconnect and paused shared-tab revocation preserve boundaries", async ({ page, browser }) => {
  test.setTimeout(120_000);
  await open(page);
  await page.getByRole("button", { name: /^Regional Joint Campaign/ }).click();
  await page.getByLabel("Game title").fill("Observer boundary exercise");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  await expect(page.getByRole("button", { name: /^Game Monitor/ })).toBeDisabled();
  const games = await (await page.request.get(backendUrl + "/v1/games")).json();
  const game = games.find((g: { title: string }) => g.title === "Observer boundary exercise");
  const prefix = `${backendUrl}/v1/games/${game.id}`;
  const hostHeaders = await headers(page);
  expect((await page.request.post(prefix + "/pause", { headers: hostHeaders, data: {} })).ok()).toBe(true);
  const context = await browser.newContext({ baseURL: page.url() }); extra.push(context);
  const guest = await context.newPage(); await open(guest);
  await guest.getByRole("button", { name: "Join game", exact: true }).click();
  await guest.getByLabel("Display name").fill("Observer guest");
  await guest.getByRole("button", { name: /Observer boundary exercise/ }).click();
  const identity = await (await guest.request.get(backendUrl + "/v1/auth/session")).json();
  await expect(guest.getByRole("button", { name: /^Game Monitor/ })).toBeDisabled();
  await page.getByRole("button", { name: "Observer access", exact: true }).click();
  await page.getByLabel("Game Monitor access").selectOption(identity.player_id);
  await expect(guest.getByRole("button", { name: /^Game Monitor/ })).toBeEnabled();
  await guest.getByRole("button", { name: /^Game Monitor/ }).click();
  await expect(guest.getByRole("heading", { name: "Ground truth — read only" })).toBeVisible({ timeout: 30000 });
  await expect(guest.locator(".truth-map canvas")).toBeVisible({ timeout: 30000 });
  const seat = await selection(guest);
  const query = `role_id=${seat.role_id}&lease_generation=${seat.lease_generation}`;
  const truth = await (await guest.request.get(`${prefix}/truth?${query}`)).json();
  const enemy = truth.units.find((u: { side: string }) => u.side === "Red");
  expect(enemy).toBeTruthy();
  await guest.getByLabel("Inspect truth object").selectOption(`unit:${enemy.id}`);
  await expect(guest.locator(".truth-inspector dd").first()).toHaveText(enemy.name);
  await expect(guest.getByRole("button", { name: "Joint planning", exact: true })).toHaveCount(0);
  await expect(guest.locator("#command-panel")).toHaveCount(0);
  // Host holds an ordinary command lease concurrently, without a truth grant.
  const roles = await (await page.request.get(prefix + "/roles")).json();
  const commander = roles.find((r: { kind: string }) => r.kind === "joint_force_commander");
  const commandLease = await (await page.request.post(`${prefix}/roles/${commander.id}/claim`, { headers: hostHeaders, data: {} })).json();
  const picture = await (await page.request.get(`${prefix}/state?role_id=${commander.id}&lease_generation=${commandLease.lease_generation}`)).json();
  expect(JSON.stringify(picture)).not.toContain(enemy.id);
  expect((await page.request.get(`${prefix}/truth?role_id=${commander.id}&lease_generation=${commandLease.lease_generation}`)).status()).toBe(403);
  expect(await guest.evaluate(() => document.documentElement.scrollHeight <= window.innerHeight + 1)).toBe(true);
  await guest.screenshot({ path: test.info().outputPath("observer-ground-truth.png"), fullPage: true });
  await guest.setViewportSize({ width: 390, height: 844 });
  await guest.getByLabel("Inspect truth object").scrollIntoViewIfNeeded();
  expect(await guest.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true);
  expect(await guest.evaluate(() => document.documentElement.scrollHeight <= window.innerHeight + 1)).toBe(true);
  await guest.screenshot({ path: test.info().outputPath("observer-ground-truth-phone.png"), fullPage: true });
  await guest.setViewportSize({ width: 1280, height: 720 });
  await guest.reload();
  await expect(guest.locator(".truth-map canvas")).toBeVisible({ timeout: 30000 });
  expect((await selection(guest)).lease_generation).toBe(seat.lease_generation);
  await context.setOffline(true);
  await expect(guest.getByRole("status")).toContainText("Connection interrupted", { timeout: 15000 });
  await context.setOffline(false);
  await expect(guest.getByRole("status")).toHaveText("Current simulation state", { timeout: 15000 });
  const tab = await context.newPage(); await tab.goto(guest.url());
  await expect(tab.locator(".truth-map canvas")).toBeVisible({ timeout: 30000 });
  await stream(tab);
  await page.getByLabel("Game Monitor access").selectOption("");
  await expect.poll(() => tab.evaluate(() => (window as any).truthStream.close)).toBe(1008);
  for (const viewer of [guest, tab]) {
    await expect(viewer.getByRole("region", { name: "Ground truth workspace" })).toHaveCount(0);
    await expect(viewer.locator(".truth-map")).toHaveCount(0);
  }
  expect((await guest.request.get(`${prefix}/truth?${query}`)).status()).toBe(403);
  expect((await guest.request.post(`${prefix}/roles/${seat.role_id}/resume`, { headers: await headers(guest), data: {} })).status()).toBe(403);
});

test("observer release and logout clear truth and close paused streams", async ({ page }) => {
  test.setTimeout(120_000);
  await open(page);
  await page.getByRole("button", { name: /^Regional Joint Campaign/ }).click();
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  await expect(page.getByRole("button", { name: "Observer access", exact: true })).toBeVisible();
  const [game] = await (await page.request.get(backendUrl + "/v1/games")).json();
  const identity = await (await page.request.get(backendUrl + "/v1/auth/session")).json();
  const prefix = `${backendUrl}/v1/games/${game.id}`;
  await page.getByRole("button", { name: "Observer access", exact: true }).click();
  await page.getByLabel("Game Controller access").selectOption(identity.player_id);
  await page.getByRole("button", { name: "Close observer access" }).click();
  expect((await page.request.post(prefix + "/pause", { headers: await headers(page), data: {} })).ok()).toBe(true);
  await expect(page.getByRole("button", { name: /^Game Controller/ })).toBeEnabled();
  await page.getByRole("button", { name: /^Game Controller/ }).click();
  await expect(page.locator(".truth-map canvas")).toBeVisible({ timeout: 30000 });
  await stream(page);
  await page.getByRole("button", { name: "Release role", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).truthStream.close)).toBe(1008);
  await expect(page.locator(".truth-map")).toHaveCount(0);
  await page.getByRole("button", { name: "Join game", exact: true }).click();
  await page.getByRole("button", { name: /Regional Joint Campaign.*paused/ }).click();
  await page.getByRole("button", { name: /^Game Controller/ }).click();
  await expect(page.locator(".truth-map canvas")).toBeVisible({ timeout: 30000 });
  await stream(page);
  expect((await page.request.post(backendUrl + "/v1/auth/logout", { headers: await headers(page), data: {} })).ok()).toBe(true);
  await expect.poll(() => page.evaluate(() => (window as any).truthStream.close)).toBe(1008);
  await expect(page.locator(".truth-map")).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  await expect(page.locator(".truth-map")).toHaveCount(0);
});


test("server restart clears an actively held observer selection", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: /^Regional Joint Campaign/ }).click();
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  await expect(page.getByRole("button", { name: "Observer access", exact: true })).toBeVisible();
  const [game] = await (await page.request.get(backendUrl + "/v1/games")).json();
  const identity = await (await page.request.get(backendUrl + "/v1/auth/session")).json();
  const prefix = `${backendUrl}/v1/games/${game.id}`;
  const seats = await (await page.request.get(prefix + "/roles")).json();
  const monitor = seats.find((s: { observer_kind?: string }) => s.observer_kind === "monitor");
  expect((await page.request.put(`${prefix}/roles/${monitor.id}/observer-grant`, { headers: await headers(page), data: { target_player_id: identity.player_id } })).ok()).toBe(true);
  expect((await page.request.post(prefix + "/pause", { headers: await headers(page), data: {} })).ok()).toBe(true);
  await expect(page.getByRole("button", { name: /^Game Monitor/ })).toBeEnabled();
  await page.getByRole("button", { name: /^Game Monitor/ }).click();
  await expect(page.locator(".truth-map canvas")).toBeVisible({ timeout: 30000 });
  expect((await selection(page)).role_id).toBe(monitor.id);
  await backend.stop(); backend = await startBackend();
  await page.reload();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  await expect(page.locator(".truth-map")).toHaveCount(0);
});
