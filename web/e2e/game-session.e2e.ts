import { expect, test, type Page, type Route } from "@playwright/test";
import { PLAYER_ID, denseSessionProjection, sessionAuthority, sessionGame, sessionProjection, sessionRole } from "./fixtures/game-session-data";

test.use({ actionTimeout: 10_000 });

type OrderBody = { player_id: string; lease_generation: number; intent: {
  intent_id: string; issuer_role: string; target: string; kind: { Move: { north_mps: number; east_mps: number } }; requested_tick: number
} };

async function openSession(page: Page, guest = false, projection = sessionProjection(), role = sessionRole(), options: {
  onLobby?: () => Promise<void>;
  waitForMap?: boolean;
} = {}) {
  const state = {
    game: sessionGame(), role, tick: 12, created: guest, projection,
    projectionFailure: false, projectionDenied: false, controlFailure: false, holdProjection: false, holdControl: false, holdSummary: false,
    projectionRequests: 0, controls: [] as string[], pendingProjections: [] as Route[], pendingControls: [] as Route[], pendingSummaries: [] as Route[],
    lostOrderResponse: false, holdOrder: false, orderDeclined: false, receiptFailure: false, receiptState: "awaiting_execution",
    orderBodies: [] as OrderBody[], submittedIntents: new Map<string, OrderBody>(), pendingOrders: [] as Route[], receiptRequests: 0,
    errors: [] as string[]
  };
  if (guest) { state.game.host_player_id = "another-player"; state.game.status = "running"; }
  page.on("pageerror", (error) => state.errors.push(error.message));
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), PLAYER_ID);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.route("**/v1/**", async (route) => {
    const request = route.request();
    const pathname = new URL(request.url()).pathname;
    const headers = {
      "access-control-allow-origin": request.headers().origin ?? "http://127.0.0.1:4173",
      "access-control-allow-credentials": "true",
      "access-control-allow-headers": "content-type, authorization, x-csrf-token",
      "access-control-allow-methods": "GET, POST, PUT, DELETE, OPTIONS"
    };
    const json = (body: unknown, status = 200) => route.fulfill({ status, headers, contentType: "application/json", body: JSON.stringify(body) });
    if (request.method() === "OPTIONS") { await route.fulfill({ status: 204, headers }); return; }
    if (pathname.startsWith("/v1/auth/")) { await json({ player_id: PLAYER_ID, display_name: "Commander", csrf_token: "fixture-csrf", expires_unix: 9999999999 }); return; }
    if (pathname.endsWith("/resume") || pathname.endsWith("/renew")) { await json(state.role); return; }
    if (pathname === "/v1/scenarios") {
      await json([{ id: "jammed-flight", title: "Jammed Flight Test", description: "Two pilot-controlled aircraft under directional jamming.", version: 1, authored_entity_count: 2, role_count: 1, requires_space_catalog: false }]); return;
    }
    if (pathname === "/v1/settings/space-catalog/status") {
      await json({ configured: false, usable: false, remembered_credentials: false, setup_auth_required: false, stale: false, syncing: false, using_cached_fallback: false, object_count: 0 }); return;
    }
    if (pathname === "/v1/games") {
      if (request.method() === "POST") {
        state.created = true;
        state.game.host_player_id = PLAYER_ID;
        await json({ game: state.game }); return;
      }
      if (state.holdSummary) { state.pendingSummaries.push(route); return; }
      await json(state.created ? [state.game] : []); return;
    }
    if (pathname.endsWith("/join")) { await json({ player_id: PLAYER_ID }); return; }
    if (pathname.endsWith("/claim")) {
      state.role.held = true; state.role.held_by_you = true; state.role.lease_generation += 1;
      await json(state.role); return;
    }
    if (pathname.endsWith("/roles")) { await json([state.role]); return; }
    if (pathname.endsWith("/authority")) { await json(sessionAuthority()); return; }
    if (pathname.endsWith("/authority/requests")) { await json([]); return; }
    if (pathname.endsWith("/start") || pathname.endsWith("/pause")) {
      expect(request.postDataJSON()).toEqual({});
      state.controls.push(pathname);
      if (state.holdControl) { state.pendingControls.push(route); return; }
      if (state.controlFailure) { await json({ error: "Unable to pause this scenario." }, 503); return; }
      state.game.status = pathname.endsWith("/pause") ? "paused" : "running";
      if (state.controls.length > 1 && state.game.status === "running") state.tick += 1;
      await json(state.game); return;
    }
    if (pathname.endsWith("/intent")) {
      const body = request.postDataJSON() as OrderBody;
      state.orderBodies.push(body);
      expect(body.player_id).toBeUndefined();
      expect(body.lease_generation).toBe(state.role.lease_generation);
      expect(body.intent.issuer_role).toBe(state.role.id);
      if (state.orderDeclined) { await json({ error: "This unit cannot accept that movement order.", code: "invalid_movement" }, 422); return; }
      state.submittedIntents.set(body.intent.intent_id, body);
      if (state.holdOrder) { state.pendingOrders.push(route); return; }
      if (state.lostOrderResponse) { state.lostOrderResponse = false; await route.abort("failed"); return; }
      await json({ status: "queued", intent_id: body.intent.intent_id, message_id: "message-" + body.intent.intent_id }); return;
    }
    if (pathname.includes("/intents/")) {
      state.receiptRequests += 1;
      const query = new URL(request.url()).searchParams;
      expect(query.get("player_id")).toBeNull();
      expect(query.get("lease_generation")).toBe(String(state.role.lease_generation));
      const body = state.submittedIntents.get(pathname.split("/").at(-1)!);
      if (!body) { await json({ error: "Order not found." }, 404); return; }
      if (state.receiptFailure) { await json({ error: "Receipt temporarily unavailable." }, 503); return; }
      await json({ intent: body.intent, state: state.receiptState, submitted_tick: 12,
        executed_tick: state.receiptState === "executed" ? 14 : undefined,
        acknowledged_tick: state.receiptState === "executed" ? 16 : undefined }); return;
    }
    if (pathname.endsWith("/state")) {
      const query = new URL(request.url()).searchParams;
      expect(query.get("player_id")).toBeNull();
      expect(query.get("role_id")).toBe(state.role.id);
      state.projectionRequests += 1;
      if (state.holdProjection) { state.pendingProjections.push(route); return; }
      if (state.projectionDenied) { await json({ error: "Role is not held by this player.", code: "role_not_held" }, 403); return; }
      await json(state.projectionFailure ? { error: "Map service temporarily unavailable." } : { ...state.projection, tick: state.tick }, state.projectionFailure ? 503 : 200); return;
    }
    if (pathname === "/v1/airports") { await json({ airports: [], total: 0 }); return; }
    await json({ error: "Unexpected test endpoint: " + pathname }, 404);
  });
  await page.goto("/");
  await options.onLobby?.();
  if (guest) {
    await page.getByRole("button", { name: "Join game", exact: true }).click();
    await page.getByRole("button", { name: /Jammed Flight Test.*running/ }).click();
  } else {
    await page.getByRole("button", { name: "Create game", exact: true }).click();
  }
  await page.getByRole("button", { name: /Blue One Pilot/ }).click();
  if (!guest) await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  if (options.waitForMap !== false) await expect(page.locator(".globe canvas")).toBeVisible();
  return state;
}

test("host pauses and resumes without replacing the map, and orders follow game state", async ({ page }, testInfo) => {
  const state = await openSession(page);
  const canvas = page.locator(".globe canvas");
  await canvas.evaluate((element) => element.setAttribute("data-retained", "yes"));
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  await expect(canvas).toHaveAttribute("data-retained", "yes");
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeDisabled();
  const beforePan = await canvas.screenshot();
  await page.keyboard.down("KeyD");
  await page.waitForTimeout(250);
  await page.keyboard.up("KeyD");
  await expect.poll(async () => (await canvas.screenshot()).equals(beforePan)).toBe(false);
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  await page.screenshot({ path: testInfo.outputPath("paused-operational-map.png") });
  await page.getByRole("button", { name: "Resume scenario", exact: true }).click();
  await expect(page.locator("header .tick")).toHaveText("TICK 13");
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeEnabled();
  await expect(canvas).toHaveAttribute("data-retained", "yes");
  expect(state.controls.map((path) => path.split("/").pop())).toEqual(["start", "pause", "start"]);
  expect(state.errors).toEqual([]);
});

test("guest follows host pause and resume while retaining the same operational map", async ({ page }) => {
  const state = await openSession(page, true);
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toHaveCount(0);
  await page.locator(".globe canvas").evaluate((element) => element.setAttribute("data-retained", "yes"));
  state.game.status = "paused";
  // Allow the two-second summary poll and software-rendered map startup in CI.
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toHaveCount(0);
  await expect(page.locator(".globe canvas")).toHaveAttribute("data-retained", "yes");
  state.game.status = "running"; state.tick = 14;
  await expect(page.locator("header .tick")).toHaveText("TICK 14", { timeout: 10_000 });
  await expect(page.getByText("Scenario paused", { exact: true })).toHaveCount(0);
  expect(state.controls).toEqual([]);
  expect(state.errors).toEqual([]);
});

test("keeps the last map during outages, disables orders, and recovers on retry", async ({ page }) => {
  const state = await openSession(page);
  await page.locator(".globe canvas").evaluate((element) => element.setAttribute("data-retained", "yes"));
  state.projectionFailure = true;
  await expect(page.getByText("Operational picture interrupted", { exact: true })).toBeVisible();
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeDisabled();
  const failedRequests = state.projectionRequests;
  await page.getByRole("button", { name: "Retry connection", exact: true }).click();
  await expect.poll(() => state.projectionRequests).toBeGreaterThan(failedRequests);
  state.projectionFailure = false; state.tick = 19;
  await expect(page.locator("header .tick")).toHaveText("TICK 19");
  await expect(page.getByText("Operational picture interrupted", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeEnabled();
  await expect(page.locator(".globe canvas")).toHaveAttribute("data-retained", "yes");
  expect(state.errors).toEqual([]);
});

test("serializes slow map reads and ignores a delayed response after leaving", async ({ page }) => {
  const state = await openSession(page);
  state.holdProjection = true;
  await expect.poll(() => state.pendingProjections.length).toBe(1);
  const requestsBeforeWait = state.projectionRequests;
  // Longer than two polling periods: a second in-flight request would be observable.
  await page.waitForTimeout(2_200);
  expect(state.projectionRequests).toBe(requestsBeforeWait);
  expect(state.pendingProjections).toHaveLength(1);
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await state.pendingProjections[0].fulfill({ contentType: "application/json", body: JSON.stringify(sessionProjection(99)) }).catch(() => undefined);
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect(page.locator("header .tick")).toHaveText("");
  await expect(page.locator(".globe")).toHaveCount(0);
  expect(state.errors).toEqual([]);
});

test("removes role data and closes inspectors when role ownership is revoked", async ({ page }) => {
  const state = await openSession(page);
  await page.getByRole("button", { name: "Map filters", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Map filters" })).toBeVisible();
  state.role.lease_generation += 1; state.role.held_by_you = false;
  await expect(page.getByText("Your role lease changed. Choose an available role to continue.", { exact: true })).toBeVisible();
  await expect(page.locator(".globe")).toHaveCount(0);
  await expect(page.getByRole("dialog", { name: "Map filters" })).toHaveCount(0);
  await expect(page.locator("header .tick")).toHaveText("");
  expect(state.errors).toEqual([]);
});

test("prevents duplicate lifecycle requests and reports a failed pause", async ({ page }) => {
  const state = await openSession(page);
  state.holdControl = true;
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Pausing…", exact: true })).toBeDisabled();
  expect(state.pendingControls).toHaveLength(1);
  await state.pendingControls[0].fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ error: "Unable to pause this scenario." }) });
  await expect(page.getByText("Scenario control failed", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toBeEnabled();
  await expect(page.locator(".globe canvas")).toBeVisible();
  expect(state.controls).toHaveLength(2);
  expect(state.errors).toEqual([]);
});

test("host can pause, resume, and leave from a narrow screen", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const state = await openSession(page);
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toBeInViewport();
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toBeInViewport();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  const mapBounds = await page.locator(".map-region").boundingBox();
  expect(mapBounds!.y + mapBounds!.height).toBeLessThanOrEqual(845);
  await expect(page.locator(".map-caption")).toBeInViewport({ ratio: 0.99 });
  await expect(page.locator(".map-controls-hint")).toBeInViewport({ ratio: 0.99 });
  expect(await page.evaluate(() => document.documentElement.scrollHeight <= window.innerHeight)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("paused-operational-map-mobile.png") });
  await page.getByRole("button", { name: "Resume scenario", exact: true }).click();
  await page.setViewportSize({ width: 844, height: 390 });
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toBeInViewport();
  const landscapeBounds = await page.locator(".map-region").boundingBox();
  expect(landscapeBounds!.y + landscapeBounds!.height).toBeLessThanOrEqual(391);
  expect(await page.evaluate(() => document.documentElement.scrollHeight <= window.innerHeight)).toBe(true);
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  expect(state.errors).toEqual([]);
});
test("clears the operational picture when the server rejects the held role", async ({ page }) => {
  const state = await openSession(page);
  state.projectionDenied = true;
  await expect(page.getByText("Your role is no longer available. Choose an available role to continue.", { exact: true })).toBeVisible();
  await expect(page.locator(".globe")).toHaveCount(0);
  await expect(page.locator("header .tick")).toHaveText("");
  const requestsAfterRevocation = state.projectionRequests;
  await page.waitForTimeout(1_100);
  expect(state.projectionRequests).toBe(requestsAfterRevocation);
  expect(state.errors).toEqual([]);
});

test("times out a hung map request and preserves the last map during recovery", async ({ page }) => {
  const state = await openSession(page);
  await page.clock.install();
  state.holdProjection = true;
  await page.clock.runFor(1_100);
  await expect.poll(() => state.pendingProjections.length).toBe(1);
  await page.clock.fastForward(10_001);
  await expect(page.getByText("Operational picture interrupted", { exact: true })).toBeVisible();
  await expect(page.getByText("The server did not respond within 10 seconds.", { exact: true })).toBeVisible();
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  await expect(page.locator(".globe canvas")).toBeVisible();
  state.holdProjection = false; state.tick = 20;
  await page.getByRole("button", { name: "Retry connection", exact: true }).click();
  await expect(page.locator("header .tick")).toHaveText("TICK 20");
  expect(state.errors).toEqual([]);
});

test("ignores a lifecycle response delivered after leaving the game", async ({ page }) => {
  const state = await openSession(page);
  state.holdControl = true;
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Pausing…", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await state.pendingControls[0].fulfill({ contentType: "application/json", body: JSON.stringify({ ...state.game, status: "paused" }) }).catch(() => undefined);
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toHaveCount(0);
  expect(state.errors).toEqual([]);
});
test("a delayed game summary cannot undo a successful host pause", async ({ page }) => {
  const state = await openSession(page);
  state.holdSummary = true;
  await expect.poll(() => state.pendingSummaries.length).toBe(1);
  const obsoleteSummary = { ...state.game, status: "running" };
  state.holdSummary = false;
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await state.pendingSummaries[0].fulfill({ contentType: "application/json", body: JSON.stringify([obsoleteSummary]) }).catch(() => undefined);
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeDisabled();
  expect(state.errors).toEqual([]);
});

test("bounds the map communications preview and updates failures and queued traffic first", async ({ page }) => {
  const state = await openSession(page, false, denseSessionProjection());
  const inspector = page.getByRole("complementary", { name: "Operational picture" });
  await expect(inspector.locator(".communication-summary")).toContainText("4,032 monitored links");
  const rows = inspector.locator(".communication-status");
  await expect(rows).toHaveCount(8);
  await expect(rows.nth(0)).toContainText("Blue One → Blue 2");
  await expect(rows.nth(0)).toContainText("Unavailable");
  await expect(rows.nth(1)).toContainText("Blue One → Blue 3");
  await expect(rows.nth(1)).toContainText("3 queued");
  await expect(inspector.getByRole("button", { name: "Inspect full network" })).toBeVisible();
  const previouslyFailed = state.projection.communication_links.find((link) => !link.available)!;
  previouslyFailed.available = true;
  await expect(rows.nth(0)).toContainText("Blue One → Blue 3");
  await expect(inspector.locator(".communication-summary")).toContainText("0 unavailable");
  await expect(rows).toHaveCount(8);
  expect(state.errors).toEqual([]);
});
test("selects an aircraft, submits the selected course, follows execution, and stops it", async ({ page }, testInfo) => {
  const projection = sessionProjection();
  const base = { ...projection.own_units[0], id: "blue-base", name: "Blue Base", domain: "Land" };
  projection.own_units.unshift(base);
  projection.own_units.push({ ...projection.own_units[1], id: "blue-two", name: "Blue Two" });
  const role = sessionRole(); role.command_units = ["blue-base", "blue-one", "blue-two"];
  const state = await openSession(page, false, projection, role);
  const orders = page.getByRole("region", { name: "Movement orders" });
  await expect(orders.getByLabel("Command unit")).toHaveValue("blue-one");
  await orders.getByLabel("Command unit").selectOption("blue-two");
  await orders.getByLabel("Course (°)").fill("90");
  await orders.getByLabel("Speed (m/s)").fill("80");
  await orders.getByRole("button", { name: "Send movement order", exact: true }).click();
  await expect.poll(() => state.orderBodies.length).toBe(1);
  expect(state.orderBodies[0].intent).toMatchObject({ target: "blue-two", kind: { Move: { north_mps: 0, east_mps: 80 } }, requested_tick: 13 });
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution");
  state.receiptState = "executed";
  await expect(orders.getByRole("status")).toContainText("Order executed at tick 14", { timeout: 10_000 });
  await expect(orders.getByRole("status")).toContainText("Blue Two · 90° at 80 m/s");
  await page.screenshot({ path: testInfo.outputPath("movement-order-executed.png") });
  await orders.getByRole("button", { name: "Stop unit", exact: true }).click();
  await expect.poll(() => state.orderBodies.length).toBe(2);
  expect(state.orderBodies[1].intent.kind.Move).toEqual({ north_mps: 0, east_mps: 0 });
  expect(state.orderBodies[1].intent.intent_id).not.toBe(state.orderBodies[0].intent.intent_id);
  await expect(orders.getByRole("status")).toContainText("Blue Two · Stopped");
  expect(state.errors).toEqual([]);
});

test("retries a lost response using the original order even after the host pauses", async ({ page }) => {
  const state = await openSession(page);
  state.lostOrderResponse = true;
  const orders = page.getByRole("region", { name: "Movement orders" });
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect(orders.getByRole("status")).toContainText("The server response was lost");
  await expect(orders.getByRole("button", { name: "Send movement order", exact: true })).toBeDisabled();
  await expect(orders.getByLabel("Course (°)")).toBeDisabled();
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await orders.getByRole("button", { name: "Retry order", exact: true }).click();
  await expect.poll(() => state.orderBodies.length).toBe(2);
  expect(state.orderBodies[1]).toEqual(state.orderBodies[0]);
  expect(state.submittedIntents.size).toBe(1);
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution");
  await expect(orders.getByRole("button", { name: "Retry order", exact: true })).toHaveCount(0);
  expect(state.errors).toEqual([]);
});

test("serializes a pending order and ignores its response after leaving", async ({ page }) => {
  const state = await openSession(page);
  state.holdOrder = true;
  const orders = page.getByRole("region", { name: "Movement orders" });
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect.poll(() => state.pendingOrders.length).toBe(1);
  await expect(orders.getByRole("button", { name: "Sending order…", exact: true })).toBeDisabled();
  await orders.locator("form").dispatchEvent("submit");
  expect(state.orderBodies.length).toBe(1);
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await state.pendingOrders[0].fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ status: "queued", message_id: "delayed-message" }) }).catch(() => undefined);
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  expect(state.receiptRequests).toBe(0);
  expect(state.errors).toEqual([]);
});

test("reports a definitive order rejection and permits a fresh command", async ({ page }) => {
  const state = await openSession(page);
  state.orderDeclined = true;
  const orders = page.getByRole("region", { name: "Movement orders" });
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect(orders.getByRole("status")).toContainText("This unit cannot accept that movement order.");
  await expect(orders.getByRole("button", { name: "Retry order", exact: true })).toHaveCount(0);
  await expect(orders.getByRole("button", { name: "Send movement order", exact: true })).toBeEnabled();
  state.orderDeclined = false;
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect.poll(() => state.orderBodies.length).toBe(2);
  expect(state.orderBodies[1].intent.intent_id).not.toBe(state.orderBodies[0].intent.intent_id);
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution");
  expect(state.errors).toEqual([]);
});

test("opens movement controls on a phone while retaining the operational map", async ({ page }, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const state = await openSession(page);
  const canvas = page.locator(".globe canvas");
  await canvas.evaluate((element) => element.setAttribute("data-retained", "yes"));
  await page.getByRole("button", { name: "Commands", exact: true }).click();
  const orders = page.getByRole("region", { name: "Movement orders" });
  await expect(orders).toBeVisible();
  await expect(canvas).toBeVisible();
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution");
  await page.screenshot({ path: testInfo.outputPath("phone-movement-controls.png") });
  await page.getByRole("button", { name: "Commands", exact: true }).click();
  await expect(orders).not.toBeVisible();
  await expect(canvas).toHaveAttribute("data-retained", "yes");
  const mapBox = await canvas.boundingBox();
  expect(mapBox!.height).toBeGreaterThan(500);
  expect(state.orderBodies.length).toBe(1);
  expect(state.errors).toEqual([]);
});
test("times out a hung order submission and retries its original command", async ({ page }) => {
  const state = await openSession(page);
  await page.clock.install();
  state.holdOrder = true;
  const orders = page.getByRole("region", { name: "Movement orders" });
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect.poll(() => state.pendingOrders.length).toBe(1);
  await page.clock.fastForward(10_100);
  await expect(orders.getByRole("status")).toContainText("The server response was lost");
  state.holdOrder = false;
  await orders.getByRole("button", { name: "Retry order", exact: true }).click();
  await expect.poll(() => state.orderBodies.length).toBe(2);
  expect(state.orderBodies[1]).toEqual(state.orderBodies[0]);
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution");
  expect(state.errors).toEqual([]);
});

test("retains the latest order receipt during an outage and recovers its execution status", async ({ page }) => {
  const state = await openSession(page);
  const orders = page.getByRole("region", { name: "Movement orders" });
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution");
  state.receiptFailure = true;
  await expect(orders.getByRole("status")).toContainText("Order status temporarily unavailable; reconnecting.", { timeout: 10_000 });
  await expect(orders.getByRole("status")).toContainText("Blue One · 0° at 130 m/s");
  state.receiptFailure = false; state.receiptState = "executed";
  await expect(orders.getByRole("status")).toContainText("Order executed at tick 14", { timeout: 10_000 });
  await expect(orders.getByRole("status")).not.toContainText("temporarily unavailable");
  expect(state.orderBodies.length).toBe(1);
  expect(state.errors).toEqual([]);
});
test("keeps an unanswered order unconfirmed and accepts a later execution reply without resubmission", async ({ page }) => {
  const state = await openSession(page);
  state.receiptState = "awaiting_acknowledgement";
  const orders = page.getByRole("region", { name: "Movement orders" });
  await orders.getByRole("button", { name: "Turn north", exact: true }).click();
  await expect(orders.getByRole("status")).toContainText("Delivered; awaiting execution confirmation");
  await expect(orders.getByRole("status")).not.toContainText("at tick");
  state.receiptState = "unconfirmed";
  await expect(orders.getByRole("status")).toContainText("Execution unconfirmed");
  await expect(orders.getByRole("status")).not.toContainText("Order rejected");
  state.receiptState = "executed";
  await expect(orders.getByRole("status")).toContainText("Order executed at tick 14");
  await expect(orders.getByRole("status")).toContainText("Confirmed at radio tick 16");
  expect(state.orderBodies).toHaveLength(1);
  expect(state.errors).toEqual([]);
});

test("opens the lobby before downloading the map and keeps host controls usable during loading", async ({ page }, testInfo) => {
  const pendingMaps: Route[] = [];
  await page.route("**/src/Globe.tsx", (route) => { pendingMaps.push(route); });
  const state = await openSession(page, false, sessionProjection(), sessionRole(), {
    waitForMap: false,
    onLobby: async () => {
      await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
      expect(pendingMaps).toHaveLength(0);
    }
  });
  await expect.poll(() => pendingMaps.length).toBe(1);
  await expect(page.getByText("Loading operational map…", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("deferred-map-loading.png") });
  await pendingMaps[0].continue();
  await expect(page.locator(".globe canvas")).toBeVisible();
  await expect(page.getByText("Loading operational map…", { exact: true })).toHaveCount(0);
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  expect(state.errors).toEqual([]);
});

test("recovers a failed map download by reloading the held game and role", async ({ page }) => {
  await page.route("**/src/Globe.tsx", (route) => route.abort("failed"));
  const state = await openSession(page, false, sessionProjection(), sessionRole(), { waitForMap: false });
  await expect(page.getByText("The operational map could not load.", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toBeEnabled();
  await page.unroute("**/src/Globe.tsx");
  await page.getByRole("button", { name: "Reload page", exact: true }).click();
  await expect(page.locator(".globe canvas")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Blue One Pilot", exact: true })).toBeVisible();
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  expect(state.controls).toHaveLength(1);
  expect(state.role.lease_generation).toBe(1);
  expect(state.errors).toEqual([]);
});

test("restores a paused held role after reload without starting the game again", async ({ page }) => {
  const state = await openSession(page);
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await page.reload();
  await expect(page.locator(".globe canvas")).toBeVisible();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeDisabled();
  expect(state.controls.map((path) => path.split("/").pop())).toEqual(["start", "pause"]);
  expect(state.role.lease_generation).toBe(1);
  expect(state.errors).toEqual([]);
});

test("does not restore a role after its ownership has changed", async ({ page }) => {
  const state = await openSession(page);
  state.role.lease_generation += 1; state.role.held_by_you = false;
  await page.reload();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect(page.locator(".globe")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  expect(state.errors).toEqual([]);
});

test("does not reopen the game after leaving and reloading", async ({ page }) => {
  const state = await openSession(page);
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await page.reload();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await expect(page.locator(".globe")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  expect(state.errors).toEqual([]);
});

test("server rejection clears a restored selection before displaying role data", async ({ page }) => {
  const state = await openSession(page);
  state.projectionDenied = true;
  await page.reload();
  await expect(page.getByText("Your role is no longer available. Choose an available role to continue.", { exact: true })).toBeVisible();
  await expect(page.locator(".globe")).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  expect(state.errors).toEqual([]);
});

test("corrupt saved selections do not block the lobby", async ({ page }) => {
  const state = await openSession(page);
  for (const value of ["{", "null", "[]"]) {
    await page.evaluate((raw) => localStorage.setItem("world-at-war-session", raw), value);
    await page.reload();
    await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
    await expect(page.locator(".globe")).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem("world-at-war-session"))).toBeNull();
  }
  expect(state.errors).toEqual([]);
});
