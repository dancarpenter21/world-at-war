import { expect, test, type Page, type Route } from "@playwright/test";
import { PLAYER_ID, sessionAuthority, sessionGame, sessionProjection, sessionRole } from "./fixtures/game-session-data";

test.use({ actionTimeout: 10_000 });

async function openSession(page: Page, guest = false) {
  const state = {
    game: sessionGame(), role: sessionRole(), tick: 12, created: guest,
    projectionFailure: false, projectionDenied: false, controlFailure: false, holdProjection: false, holdControl: false, holdSummary: false,
    projectionRequests: 0, controls: [] as string[], pendingProjections: [] as Route[], pendingControls: [] as Route[], pendingSummaries: [] as Route[],
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
      "access-control-allow-headers": "content-type, authorization",
      "access-control-allow-methods": "GET, POST, PUT, DELETE, OPTIONS"
    };
    const json = (body: unknown, status = 200) => route.fulfill({ status, headers, contentType: "application/json", body: JSON.stringify(body) });
    if (request.method() === "OPTIONS") { await route.fulfill({ status: 204, headers }); return; }
    if (pathname === "/v1/scenarios") {
      await json([{ id: "jammed-flight", title: "Jammed Flight Test", description: "Two pilot-controlled aircraft under directional jamming.", version: 1, authored_entity_count: 2, role_count: 1, requires_space_catalog: false }]); return;
    }
    if (pathname === "/v1/settings/space-catalog/status") {
      await json({ configured: false, usable: false, remembered_credentials: false, setup_auth_required: false, stale: false, syncing: false, using_cached_fallback: false, object_count: 0 }); return;
    }
    if (pathname === "/v1/games") {
      if (request.method() === "POST") {
        state.created = true;
        state.game.host_player_id = request.postDataJSON().host_player_id;
        await json({ game: state.game }); return;
      }
      if (state.holdSummary) { state.pendingSummaries.push(route); return; }
      await json(state.created ? [state.game] : []); return;
    }
    if (pathname.endsWith("/join")) { await json({ player_id: PLAYER_ID }); return; }
    if (pathname.endsWith("/claim")) {
      state.role.held = true; state.role.lease_generation += 1;
      await json(state.role); return;
    }
    if (pathname.endsWith("/roles")) { await json([state.role]); return; }
    if (pathname.endsWith("/authority")) { await json(sessionAuthority()); return; }
    if (pathname.endsWith("/authority/requests")) { await json([]); return; }
    if (pathname.endsWith("/start") || pathname.endsWith("/pause")) {
      expect(request.postDataJSON()).toEqual({ player_id: PLAYER_ID });
      state.controls.push(pathname);
      if (state.holdControl) { state.pendingControls.push(route); return; }
      if (state.controlFailure) { await json({ error: "Unable to pause this scenario." }, 503); return; }
      state.game.status = pathname.endsWith("/pause") ? "paused" : "running";
      if (state.controls.length > 1 && state.game.status === "running") state.tick += 1;
      await json(state.game); return;
    }
    if (pathname.endsWith("/state")) {
      const query = new URL(request.url()).searchParams;
      expect(query.get("player_id")).toBe(PLAYER_ID);
      expect(query.get("role_id")).toBe(state.role.id);
      state.projectionRequests += 1;
      if (state.holdProjection) { state.pendingProjections.push(route); return; }
      if (state.projectionDenied) { await json({ error: "Role is not held by this player.", code: "role_not_held" }, 403); return; }
      await json(state.projectionFailure ? { error: "Map service temporarily unavailable." } : sessionProjection(state.tick), state.projectionFailure ? 503 : 200); return;
    }
    if (pathname === "/v1/airports") { await json({ airports: [], total: 0 }); return; }
    await json({ error: "Unexpected test endpoint: " + pathname }, 404);
  });
  await page.goto("/");
  if (guest) {
    await page.getByRole("button", { name: "Join game", exact: true }).click();
    await page.getByRole("button", { name: /Jammed Flight Test.*running/ }).click();
  } else {
    await page.getByRole("button", { name: "Create game", exact: true }).click();
  }
  await page.getByRole("button", { name: /Blue One Pilot/ }).click();
  if (!guest) await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.locator("header .tick")).toHaveText("TICK 12");
  await expect(page.locator(".globe canvas")).toBeVisible();
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
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toHaveCount(0);
  await expect(page.locator(".globe canvas")).toHaveAttribute("data-retained", "yes");
  state.game.status = "running"; state.tick = 14;
  await expect(page.locator("header .tick")).toHaveText("TICK 14");
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

test("removes role data and closes inspectors when the role lease changes", async ({ page }) => {
  const state = await openSession(page);
  await page.getByRole("button", { name: "Map filters", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Map filters" })).toBeVisible();
  state.role.lease_generation += 1;
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