import { expect, test, type Browser, type BrowserContext, type Page, type WebSocketRoute } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { startBackend, backendUrl, type Backend } from "./support/server";
import type { PlanningView } from "../src/PlanningWorkspace";

const baseURL = `http://${process.env.E2E_BROWSER_HOST ?? "127.0.0.1"}:4173`;
const commanderRole = "00000000-0000-0000-0000-0000000000c9";
const pilotRole = "00000000-0000-0000-0000-0000000000d3";
const controllerRole = "00000000-0000-0000-0000-0000000000cd";
const sessionKey = "world-at-war-session";
let backend: Backend;
let contexts: BrowserContext[];

test.beforeEach(async () => { contexts = []; backend = await startBackend(); });
test.afterEach(async ({}, info) => {
  if (info.status !== info.expectedStatus && backend) await info.attach("backend.log", { body: backend.output(), contentType: "text/plain" });
  try { await Promise.all(contexts.map(context => context.close())); }
  finally { await backend?.stop(); }
});

async function player(browser: Browser) {
  const context = await browser.newContext({ baseURL }); contexts.push(context);
  // Replace only external background imagery; all application HTTP/WebSocket traffic is real.
  await context.route("https://tile.openstreetmap.org/**", route => route.fulfill({ contentType: "image/png", body: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==", "base64") }));
  const page = await context.newPage(); await page.goto("/"); return page;
}
async function session(page: Page) {
  return page.evaluate(key => JSON.parse(localStorage.getItem(key)!) as { player_id: string; game_id: string; role_id: string }, sessionKey);
}
async function api(page: Page, path: string, data?: unknown) {
  return page.evaluate(async ({ url, data }) => {
    const endpoint = new URL(url);
    endpoint.searchParams.delete("player_id");
    const selected = JSON.parse(localStorage.getItem("world-at-war-session") ?? "null");
    if (endpoint.searchParams.has("role_id")) endpoint.searchParams.set("lease_generation", String(selected?.lease_generation ?? 0));
    const session = await (await fetch(new URL("/v1/auth/session", url), { credentials: "include" })).json();
    if (data && typeof data === "object") { delete (data as Record<string, unknown>).player_id; delete (data as Record<string, unknown>).host_player_id; }
    const response = await fetch(endpoint, { credentials: "include", ...(data === undefined ? {} : { method: "POST", headers: { "content-type": "application/json", "x-csrf-token": session.csrf_token }, body: JSON.stringify(data) }) });
    return { status: response.status, body: await response.json() };
  }, { url: `${backendUrl}${path}`, data });
}
async function planning(page: Page): Promise<PlanningView> {
  const s = await session(page);
  const result = await api(page, `/v1/games/${s.game_id}/planning?player_id=${s.player_id}&role_id=${s.role_id}`);
  expect(result.status).toBe(200); return result.body;
}
async function create(page: Page, title: string) {
  await page.getByRole("button", { name: /Regional Joint Campaign/ }).click();
  await page.getByLabel("Game title").fill(title);
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  await page.getByRole("button", { name: /Joint Force Commander/ }).click();
  await expect.poll(async () => (await session(page))?.role_id).toBe(commanderRole);
}
async function join(page: Page, title: string, role: string) {
  await page.getByRole("button", { name: "Join game", exact: true }).click();
  await page.getByRole("button", { name: new RegExp(title) }).click();
  await page.getByRole("button", { name: new RegExp(role) }).click();
  await expect.poll(async () => ((await session(page))?.role_id ?? "")).toMatch(/^[0-9a-f-]{36}$/);
}
async function openPlanning(page: Page) {
  await page.getByRole("button", { name: "Joint planning", exact: true }).click();
  await expect(page.getByRole("region", { name: "Joint campaign planning" })).toBeVisible();
}
async function control(page: Page, action: "pause" | "start") {
  const s = await session(page);
  expect((await api(page, `/v1/games/${s.game_id}/${action}`, { player_id: s.player_id })).status).toBe(200);
}
async function roles(page: Page) { const s = await session(page); return (await api(page, `/v1/games/${s.game_id}/roles`)).body as { id: string; lease_generation: number }[]; }

test("real players receive published plans only after delivery and cannot read another role's picture", async ({ browser }) => {
  test.setTimeout(180_000);
  const commander = await player(browser), pilot = await player(browser), controller = await player(browser);
  await create(commander, "Browser delivery campaign");
  await join(pilot, "Browser delivery campaign", "Intercept Pilot");
  await join(controller, "Browser delivery campaign", "West Sector Controller");
  const c = await session(commander), p = await session(pilot);
  expect(c.player_id).not.toBe(p.player_id);
  expect((await api(pilot, `/v1/games/${c.game_id}/roles/${commanderRole}/claim`, { player_id: p.player_id })).status).toBe(409);
  await commander.getByRole("button", { name: "Start scenario", exact: true }).click();
  for (const page of [commander, pilot, controller]) await openPlanning(page);
  await control(commander, "pause");
  await commander.getByLabel("Commander intent", { exact: true }).fill("PRIVATE-DRAFT: protect the browser campaign.");
  await commander.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Draft saved");
  await expect(pilot.getByText("No planning product has reached this role.", { exact: false })).toBeVisible();
  for (const page of [pilot, controller]) {
    expect(JSON.stringify(await planning(page))).not.toContain("PRIVATE-DRAFT");
  }
  await commander.getByRole("button", { name: "Approve and publish selected course", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Message queued", { timeout: 20_000 });
  expect((await planning(pilot)).received).toBeNull();
  const networkPath = `/v1/games/${c.game_id}/network?player_id=${p.player_id}&role_id=${pilotRole}`;
  expect(JSON.stringify((await api(pilot, networkPath)).body)).not.toContain("PRIVATE-DRAFT");
  for (const endpoint of ["state", "planning", "network"]) {
    expect((await api(pilot, `/v1/games/${c.game_id}/${endpoint}?player_id=${p.player_id}&role_id=${commanderRole}`)).status).toBe(403);
  }
  const lease = (await roles(commander)).find(role => role.id === commanderRole)!.lease_generation;
  expect((await api(pilot, `/v1/games/${c.game_id}/roles/${commanderRole}/planning`, { player_id: p.player_id, lease_generation: lease, action: "publish", expected_revision: 2, course_id: (await planning(commander)).draft!.courses[0].id })).status).toBe(403);
  const picture = await api(pilot, `/v1/games/${c.game_id}/state?player_id=${p.player_id}&role_id=${pilotRole}`);
  for (const suffix of ["033", "03d", "03e"]) expect(JSON.stringify(picture.body)).not.toContain(`00000000-0000-0000-0000-000000000${suffix}`);
  await control(commander, "start");
  for (const page of [pilot, controller]) {
    await expect.poll(async () => (await planning(page)).received?.revision, { timeout: 100_000 }).toBe(2);
    await expect(page.getByLabel("Commander intent", { exact: true })).toHaveValue("PRIVATE-DRAFT: protect the browser campaign.");
    await expect(page.getByLabel("Commander intent", { exact: true })).toBeDisabled();
    await expect(page.getByText("Import airspace order", { exact: true })).toHaveCount(0);
  }
  await commander.screenshot({ path: "test-results/campaign-commander.png", fullPage: true });
  await pilot.screenshot({ path: "test-results/campaign-pilot.png", fullPage: true });
});

test("reload restores the held role without changing its lease and a severed network stream resynchronizes", async ({ browser }) => {
  test.setTimeout(90_000);
  const page = await player(browser);
  await create(page, "Browser reconnect campaign");
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Joint planning", exact: true })).toBeVisible();
  const before = await session(page), lease = (await roles(page)).find(role => role.id === commanderRole)!.lease_generation;
  let disconnected = false;
  const sockets: WebSocketRoute[] = [];
  const frames: { sequence: number; resync: boolean }[] = [];
  await page.routeWebSocket(/\/network\/stream/, socket => {
    sockets.push(socket);
    if (disconnected) { socket.close(); return; }
    const server = socket.connectToServer();
    server.onMessage(message => { frames.push(JSON.parse(String(message))); socket.send(message); });
  });
  await page.reload();
  await expect(page.getByRole("heading", { name: "Joint Force Commander", exact: true })).toBeVisible();
  expect(await session(page)).toEqual(before);
  expect((await roles(page)).find(role => role.id === commanderRole)!.lease_generation).toBe(lease);
  await openPlanning(page);
  await page.getByLabel("Commander intent", { exact: true }).fill("Recovered commander can still save.");
  await page.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(page.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Draft saved");
  await page.getByRole("button", { name: "Close planning", exact: true }).click();
  await page.route("**/v1/**", route => route.abort("internetdisconnected"));
  await page.reload();
  await expect(page.getByText(/Connection unavailable; retrying/)).toBeVisible();
  expect(await session(page)).toEqual(before);
  await page.unroute("**/v1/**");
  await expect(page.getByRole("heading", { name: "Joint Force Commander", exact: true })).toBeVisible({ timeout: 15_000 });
  expect((await roles(page)).find(role => role.id === commanderRole)!.lease_generation).toBe(lease);
  await page.getByRole("button", { name: "Network", exact: true }).click();
  await expect.poll(() => frames.length).toBeGreaterThan(0);
  const previous = frames.at(-1)!.sequence;
  disconnected = true; sockets.at(-1)!.close();
  await expect.poll(async () => (await planning(page)).tick, { timeout: 15_000 }).toBeGreaterThan(previous + 2);
  disconnected = false;
  await expect.poll(() => frames.some(frame => frame.resync && frame.sequence > previous + 1), { timeout: 15_000 }).toBe(true);
  expect(sockets.slice(1).some(socket => new URL(socket.url()).searchParams.has("after_sequence"))).toBe(true);
  await expect(page.locator(".network-header")).toContainText("TICK");
  await page.getByRole("button", { name: "Back to map", exact: true }).click();
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await page.reload();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  expect(await page.evaluate(key => localStorage.getItem(key), sessionKey)).toBeNull();
});

async function importSource(page: Page, source: string) {
  await page.getByText("Import airspace order", { exact: true }).click();
  await page.getByLabel("ACO text file").setInputFiles({ name: "training.aco", mimeType: "text/plain", buffer: Buffer.from(source) });
  await page.getByLabel("UTC anchor", { exact: true }).fill("2026-09-01T00:00:00Z");
  await page.getByLabel("ACO year", { exact: true }).fill("2026");
  await page.getByLabel("UTC horizon", { exact: true }).fill("2026-09-03T00:00:00Z");
  await page.getByRole("button", { name: "Preview airspace import", exact: true }).click();
  await expect(page.getByRole("group", { name: "Resolve TRAINING", exact: true })).toBeVisible();
}
async function resolveImport(page: Page) {
  const record = page.getByRole("group", { name: "Resolve TRAINING", exact: true });
  await record.getByLabel("Assigned controller").selectOption(controllerRole);
  await record.getByLabel("Airspace kind").selectOption("patrol");
  await page.getByRole("button", { name: "Preview airspace import", exact: true }).click();
  await expect(page.getByText("Preview valid.", { exact: true })).toBeVisible();
}

test("ACO preview, resolutions, atomic apply, deduplication and publication use the real backend", async ({ browser }) => {
  test.setTimeout(180_000);
  const commander = await player(browser), pilot = await player(browser);
  await create(commander, "Browser airspace campaign"); await join(pilot, "Browser airspace campaign", "Intercept Pilot");
  await commander.getByRole("button", { name: "Start scenario", exact: true }).click();
  await openPlanning(commander); await openPlanning(pilot); await control(commander, "pause");
  const source = await readFile(new URL("../../data/aco/labelled.aco", import.meta.url), "utf8");
  await importSource(commander, source.replace("0AMSL-10000AMSL", "SFC-FL100"));
  await expect(commander.getByRole("button", { name: "Apply airspaces to draft", exact: true })).toBeDisabled();
  expect((await planning(commander)).draft!.revision).toBe(1);
  const record = commander.getByRole("group", { name: "Resolve TRAINING", exact: true });
  await record.getByLabel("Resolved floor MSL (m)", { exact: true }).fill("0");
  await record.getByLabel("Resolved ceiling MSL (m)", { exact: true }).fill("3048");
  await record.getByLabel("Resolved activation ticks").fill("0-600, 900-1200");
  await resolveImport(commander);
  await commander.getByRole("button", { name: "Apply airspaces to draft", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Airspaces applied to draft");
  const imported = (await planning(commander)).draft!;
  expect(imported.revision).toBe(2); expect(imported.published_tick).toBeNull();
  const volume = imported.airspaces.find(volume => volume.name === "TRAINING")!;
  expect(volume.active_periods).toEqual([{ start_tick: 0, end_tick: 600 }, { start_tick: 900, end_tick: 1200 }]);
  expect(volume.source?.external_id).toBe("TRAINING");
  expect((await planning(pilot)).received).toBeNull();
  await commander.getByRole("button", { name: "Preview airspace import", exact: true }).click();
  await expect(commander.getByText("TRAINING: unchanged", { exact: true })).toBeVisible();
  await commander.getByRole("button", { name: "Apply airspaces to draft", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Airspaces applied to draft");
  expect((await planning(commander)).draft!.revision).toBe(2);
  expect((await planning(commander)).draft!.airspaces.find(item => item.name === "TRAINING")!.id).toBe(volume.id);
  // A normal save must preserve source geometry, provenance and disjoint activation periods.
  await commander.getByLabel("Commander intent", { exact: true }).fill("Publish imported training airspace.");
  await commander.getByRole("button", { name: "Save draft", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status").filter({ hasText: "Draft saved" })).toBeVisible();
  expect((await planning(commander)).draft!.airspaces.find(item => item.id === volume.id)).toEqual(volume);
  await commander.getByRole("button", { name: "Approve and publish selected course", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status").filter({ hasText: "Message queued" })).toBeVisible();
  expect((await planning(pilot)).received).toBeNull(); await control(commander, "start");
  await expect.poll(async () => (await planning(pilot)).received?.revision, { timeout: 100_000 }).toBe(3);
  await expect(pilot.getByRole("cell", { name: "TRAINING", exact: true })).toBeVisible();
  await expect(pilot.getByRole("cell", { name: "0–600, 900–1200", exact: true })).toBeVisible();
  await commander.screenshot({ path: "test-results/airspace-import.png", fullPage: true });
});

test("invalid imports, excluded records and stale previews cannot overwrite a campaign draft", async ({ browser }) => {
  const page = await player(browser); await create(page, "Browser import conflicts");
  await page.getByRole("button", { name: "Start scenario", exact: true }).click(); await openPlanning(page);
  const source = await readFile(new URL("../../data/aco/labelled.aco", import.meta.url), "utf8");
  await importSource(page, source);
  const s = await session(page), lease = (await roles(page)).find(role => role.id === commanderRole)!.lease_generation;
  const payload = { player_id: s.player_id, lease_generation: lease, expected_revision: 1, source,
    options: { anchor_utc: "2026-09-01T00:00:00Z", anchor_tick: 0, year: 2026, horizon_end_utc: "2026-09-03T00:00:00Z", resolutions: {} } };
  expect((await api(page, `/v1/games/${s.game_id}/roles/${commanderRole}/planning/aco/apply`, payload)).status).toBe(422);
  expect((await planning(page)).draft!.airspaces).toHaveLength(2);
  await page.getByLabel("Exclude TRAINING", { exact: true }).check();
  await page.getByRole("button", { name: "Preview airspace import", exact: true }).click();
  await expect(page.getByText("TRAINING: excluded", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Apply airspaces to draft", exact: true }).click();
  await expect(page.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Airspaces applied to draft");
  expect((await planning(page)).draft!.revision).toBe(1);
  await page.getByLabel("Exclude TRAINING", { exact: true }).uncheck(); await resolveImport(page);
  const draft = (await planning(page)).draft!;
  // Simulate a competing editor through the same public API, without fabricating any response.
  expect((await api(page, `/v1/games/${s.game_id}/roles/${commanderRole}/planning`, { player_id: s.player_id, lease_generation: lease, action: "save", expected_revision: 1, plan: { ...draft, intent: "Concurrent editor saved this." } })).status).toBe(200);
  const stale = { ...payload, options: { ...payload.options, resolutions: { TRAINING: { controller_role_id: controllerRole, kind: "patrol" } } } };
  expect((await api(page, `/v1/games/${s.game_id}/roles/${commanderRole}/planning/aco/apply`, stale)).status).toBe(422);
  await expect(page.getByText("Preview outdated. Preview again before applying.", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Apply airspaces to draft", exact: true })).toBeDisabled();
  expect((await planning(page)).draft!.intent).toBe("Concurrent editor saved this.");
  expect((await planning(page)).draft!.airspaces).toHaveLength(2);
  await page.getByRole("button", { name: "Preview airspace import", exact: true }).click();
  await expect(page.getByText("Preview valid.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Apply airspaces to draft", exact: true }).click();
  await expect(page.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Airspaces applied to draft");
  expect((await planning(page)).draft!.revision).toBe(3);
});

test("missing games and lost role ownership discard saved sessions", async ({ browser }) => {
  const page = await player(browser); await create(page, "Browser stale sessions");
  const s = await session(page);
  await page.evaluate(({ key, s }) => localStorage.setItem(key, JSON.stringify({ ...s, role_id: "00000000-0000-0000-0000-0000000000d3" })), { key: sessionKey, s });
  await page.reload(); await expect(page.getByText("Previous role is unavailable. Choose a scenario.")).toBeVisible();
  expect(await page.evaluate(key => localStorage.getItem(key), sessionKey)).toBeNull();
  await page.evaluate(({ key, s }) => localStorage.setItem(key, JSON.stringify({ ...s, game_id: "00000000-0000-0000-0000-000000000001" })), { key: sessionKey, s });
  await page.reload(); await expect(page.getByText("Previous scenario is unavailable. Choose a scenario.")).toBeVisible();
  expect(await page.evaluate(key => localStorage.getItem(key), sessionKey)).toBeNull();
});

test("diagnostics measure the held role, distinguish pause and recover after state request failure", async ({ browser }) => {
  test.setTimeout(60_000);
  const page = await player(browser);
  await create(page, "Browser diagnostics campaign");
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await page.getByRole("button", { name: "Diagnostics", exact: true }).click();
  const panel = page.getByRole("region", { name: "Simulation diagnostics" });
  await expect(panel.getByRole("status")).toHaveText("Connection: Connected");
  await expect(panel).toContainText("KiB (uncompressed)");
  const s = await session(page);
  const path = `/v1/games/${s.game_id}/diagnostics?player_id=${s.player_id}&role_id=${s.role_id}`;
  await expect.poll(async () => (await api(page, path)).body.server_tick_ms.samples).toBeGreaterThan(0);
  expect((await api(page, `/v1/games/${s.game_id}/diagnostics?player_id=${s.player_id}&role_id=${pilotRole}`)).status).toBe(403);
  await control(page, "pause");
  await expect(panel).toContainText("Simulation is paused; a stationary tick is expected.");
  const stateText = await page.evaluate(async ({ base, game, role }) => { const saved = JSON.parse(localStorage.getItem("world-at-war-session")!); return (await fetch(`${base}/v1/games/${game}/state?role_id=${role}&lease_generation=${saved.lease_generation}`, { credentials: "include" })).text(); }, { base: backendUrl, game: s.game_id, role: s.role_id });
  const state = JSON.parse(stateText);
  const diagnostics = (await api(page, path)).body;
  expect(diagnostics.projection.tick).toBe(state.tick);
  expect(diagnostics.projection.visible_queue_packets).toBe(state.communication_links.reduce((sum: number, link: { queued_packets: number }) => sum + link.queued_packets, 0));
  expect(diagnostics.projection.snapshot_bytes).toBe(Buffer.byteLength(stateText));
  expect(Object.keys(diagnostics.projection).sort()).toEqual(["build_ms", "serialization_ms", "snapshot_bytes", "tick", "visible_queue_bytes", "visible_queue_packets"]);
  await page.route("**/state?*", route => route.abort("internetdisconnected"));
  await expect(panel.getByRole("status")).toHaveText("Connection: Reconnecting");
  await page.unroute("**/state?*");
  await expect(panel.getByRole("status")).toHaveText("Connection: Connected");
  await control(page, "start");
  await expect(panel).toContainText("running · tick");
  await expect(panel).not.toContainText("Simulation is paused");
  await page.screenshot({ path: "test-results/campaign-diagnostics.png", fullPage: true });
});

async function planningAction(page: Page, action: Record<string, unknown>) {
  const s = await session(page);
  const lease = (await roles(page)).find(role => role.id === s.role_id)!.lease_generation;
  return api(page, `/v1/games/${s.game_id}/roles/${s.role_id}/planning`, { player_id: s.player_id, lease_generation: lease, ...action });
}

test("clearances, handoffs and cancellation wait for delivery and enforce controller authority", async ({ browser }) => {
  test.setTimeout(240_000);
  const commander = await player(browser), pilot = await player(browser);
  const west = await player(browser), east = await player(browser);
  await create(commander, "Browser airspace execution");
  await join(pilot, "Browser airspace execution", "Intercept Pilot");
  await join(west, "Browser airspace execution", "West Sector Controller");
  await join(east, "Browser airspace execution", "East Sector Controller");
  await commander.getByRole("button", { name: "Start scenario", exact: true }).click();
  await openPlanning(commander);
  await commander.getByRole("button", { name: "Approve and publish selected course", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Message queued", { timeout: 20_000 });
  for (const page of [pilot, west, east]) {
    await expect.poll(async () => (await planning(page)).received?.revision, { timeout: 90_000 }).toBe(1);
    await openPlanning(page);
  }
  await control(commander, "pause");
  const plan = (await planning(pilot)).received!;
  const mission = plan.courses.find(course => course.id === plan.selected_course_id)!.missions[0];
  const westSector = plan.airspaces.find(space => space.controller_role_id === controllerRole)!;
  const eastSector = plan.airspaces.find(space => space.controller_role_id !== controllerRole)!;
  await pilot.getByRole("combobox", { name: "Aircraft", exact: true }).selectOption(mission.unit_id);
  await pilot.getByRole("combobox", { name: "Destination airspace", exact: true }).selectOption(westSector.id);
  await pilot.getByRole("button", { name: "Request clearance", exact: true }).click();
  await expect(pilot.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Message queued", { timeout: 20_000 });
  expect((await planning(west)).clearances).toHaveLength(0);
  expect((await planning(pilot)).clearances).toHaveLength(0);
  await control(commander, "start");
  await expect.poll(async () => (await planning(west)).clearances.length, { timeout: 30_000 }).toBe(1);
  await control(commander, "pause");
  const clearance = (await planning(west)).clearances[0];
  expect((await planningAction(east, { action: "grant_clearance", clearance })).status).toBe(422);
  expect((await planningAction(pilot, { action: "grant_clearance", clearance })).status).toBe(422);
  await west.getByRole("button", { name: "Approve request", exact: true }).click();
  await expect(west.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Message queued", { timeout: 20_000 });
  expect((await planning(pilot)).clearances).toHaveLength(0);
  await control(commander, "start");
  await expect.poll(async () => (await planning(pilot)).clearances.some(item => item.id === clearance.id), { timeout: 30_000 }).toBe(true);
  await expect(pilot.getByText(`Clearance · ${westSector.name}`, { exact: false })).toBeVisible();

  await control(commander, "pause");
  await west.getByRole("combobox", { name: "Aircraft", exact: true }).selectOption(mission.unit_id);
  await west.getByRole("combobox", { name: "Destination airspace", exact: true }).selectOption(eastSector.id);
  await west.getByRole("button", { name: "Offer handoff", exact: true }).click();
  await expect(west.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Message queued", { timeout: 20_000 });
  const handoff = (await planning(west)).handoffs[0];
  expect((await planning(east)).handoffs).toHaveLength(0);
  expect((await planning(pilot)).handoffs).toHaveLength(0);
  expect((await planningAction(pilot, { action: "offer_handoff", handoff: { ...handoff, from_role_id: pilotRole } })).status).toBe(422);
  await control(commander, "start");
  await expect(east.getByRole("button", { name: "Accept handoff", exact: true })).toBeVisible({ timeout: 30_000 });
  await control(commander, "pause");
  expect((await planningAction(west, { action: "accept_handoff", handoff_id: handoff.id })).status).toBe(422);
  await east.getByRole("button", { name: "Accept handoff", exact: true }).click();
  await expect(east.getByText(/Accepted; awaiting aircraft receipt/)).toBeVisible();
  expect((await planningAction(east, { action: "accept_handoff", handoff_id: handoff.id })).status).toBe(422);
  expect((await planning(pilot)).handoffs).toHaveLength(0);
  await control(commander, "start");
  await expect(pilot.getByText(/Received by aircraft/)).toBeVisible({ timeout: 30_000 });
  await expect(west.getByText(/Received by aircraft/)).toBeVisible({ timeout: 30_000 });
  await expect(east.getByText(/Received by aircraft/)).toBeVisible({ timeout: 30_000 });

  await control(commander, "pause");
  expect((await planningAction(west, { action: "cancel", mission_ids: [mission.id] })).status).toBe(422);
  const report = commander.locator("article.planning-card").filter({ has: commander.getByRole("button", { name: "Send cancellation", exact: true }) }).filter({ hasText: mission.name });
  await expect(report).toHaveCount(1);
  await report.getByRole("button", { name: "Send cancellation", exact: true }).click();
  await expect(commander.getByRole("region", { name: "Joint campaign planning" }).getByRole("status")).toContainText("Message queued", { timeout: 20_000 });
  expect((await planning(pilot)).reports.find(item => item.mission_id === mission.id)?.state).not.toBe("cancelled");
  await control(commander, "start");
  await expect.poll(async () => (await planning(pilot)).reports.find(item => item.mission_id === mission.id)?.state, { timeout: 30_000 }).toBe("cancelled");
  await expect(report).toContainText("cancelled", { timeout: 30_000 });
  await pilot.screenshot({ path: "test-results/campaign-handoff-cancellation.png", fullPage: true });
  const canvas = pilot.locator(".globe canvas");
  await canvas.evaluate(element => element.setAttribute("data-retained", "planning"));
  await pilot.getByRole("button", { name: "Close planning", exact: true }).click();
  await expect(canvas).toBeVisible();
  await expect(canvas).toHaveAttribute("data-retained", "planning");
  await pilot.bringToFront();
  const beforePan = await canvas.screenshot();
  await pilot.keyboard.down("KeyD");
  try {
    await expect.poll(async () => (await canvas.screenshot()).equals(beforePan), { timeout: 20_000 }).toBe(false);
  } finally {
    await pilot.keyboard.up("KeyD");
  }
});
