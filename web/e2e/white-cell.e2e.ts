import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { backendUrl, startBackend, type Backend } from "./support/server";
let backend: Backend;
let contexts: BrowserContext[];
test.beforeEach(async () => { contexts = []; backend = await startBackend(); });
test.afterEach(async ({}, info) => {
  if (info.status !== info.expectedStatus && backend) await info.attach("backend.log", { body: backend.output(), contentType: "text/plain" });
  await Promise.all(contexts.map(context => context.close())); await backend?.stop();
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
async function join(page: Page, name: string, game: string) {
  await open(page); await page.getByRole("button", { name: "Join game", exact: true }).click();
  await page.getByLabel("Display name").fill(name); await page.getByRole("button", { name: new RegExp(`^${game}`) }).click();
  await expect(page.getByRole("region", { name: "Team and players" })).toBeVisible();
}

test("White Cell hosts human Red and Blue, approves a late referee, shares notes and controls speed", async ({ page, browser }, info) => {
  test.setTimeout(120_000);
  await open(page); await page.getByRole("button", { name: /^Regional Joint Campaign/ }).click(); await page.getByLabel("Join as").selectOption("white_cell");
  await page.getByLabel("Red control").selectOption("human"); await page.getByLabel("Game title").fill("White Cell exercise");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  await expect(page.getByRole("region", { name: "White Cell workspace" })).toBeVisible({ timeout: 30_000 });
  await expect(page.locator(".truth-map canvas")).toBeVisible();
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toBeVisible();
  const seat = await selection(page); const prefix = `${backendUrl}/v1/games/${seat.game_id}`;
  const blueContext = await browser.newContext(); const redContext = await browser.newContext(); const refereeContext = await browser.newContext();
  contexts.push(blueContext, redContext, refereeContext);
  const blue = await blueContext.newPage(), red = await redContext.newPage(), referee = await refereeContext.newPage();
  await join(blue, "Blue commander", "White Cell exercise"); await blue.locator(".team-card").filter({ hasText: /^Blue/ }).click();
  await blue.getByRole("button", { name: /^Joint Force Commander/ }).click(); await expect(blue.locator(".globe canvas")).toBeVisible();
  await join(red, "Red commander", "White Cell exercise"); await red.locator(".team-card").filter({ hasText: /^Red/ }).click();
  await red.getByRole("button", { name: /^Red Commander/ }).click(); await expect(red.locator(".globe canvas")).toBeVisible();
  await join(referee, "Late referee", "White Cell exercise"); await referee.locator(".team-card").filter({ hasText: /^White Cell/ }).click();
  await expect(referee.getByText("Waiting for approval to join White Cell.")).toBeVisible();
  expect((await referee.request.get(`${prefix}/white-cell?role_id=${seat.role_id}&lease_generation=${seat.lease_generation}`)).status()).toBe(403);
  await page.getByRole("button", { name: /^Team & players/ }).click();
  await page.locator(".team-request").filter({ hasText: "Late referee" }).getByRole("button", { name: "Approve", exact: true }).click();
  await page.getByRole("button", { name: "Close team panel" }).click();
  await expect(referee.getByRole("region", { name: "White Cell workspace" })).toBeVisible({ timeout: 30_000 });
  await referee.getByRole("combobox", { name: "Simulation speed", exact: true }).selectOption("5");
  await expect(page.getByRole("combobox", { name: "Simulation speed", exact: true })).toHaveValue("5");
  await referee.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toBeVisible();
  await referee.getByRole("combobox", { name: "Simulation speed", exact: true }).selectOption("0.25");
  await expect(page.getByRole("combobox", { name: "Simulation speed", exact: true })).toHaveValue("0.25");
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toBeVisible();
  await referee.getByRole("button", { name: "Team notebook", exact: true }).click();
  await referee.getByLabel("Referee note", { exact: true }).fill("Hold the exercise for referee review.");
  await referee.getByRole("button", { name: "Share note", exact: true }).click();
  await page.getByRole("button", { name: "Team notebook", exact: true }).click();
  await expect(page.locator(".referee-notes")).toContainText("Hold the exercise for referee review.");
  await page.screenshot({ path: info.outputPath("white-cell-notebook.png"), fullPage: true });
  const captured = await selection(referee);
  const resumedTab = await refereeContext.newPage(); await resumedTab.goto("/");
  await expect(resumedTab.getByRole("region", { name: "White Cell workspace" })).toBeVisible({ timeout: 30_000 });
  await resumedTab.evaluate(({ base, seat }) => {
    const url = new URL(`${base}/v1/games/${seat.game_id}/truth/stream`);
    url.protocol = "ws:";
    url.search = new URLSearchParams({ role_id: seat.role_id, lease_generation: String(seat.lease_generation) }).toString();
    const socket = new WebSocket(url);
    const result = { frames: 0, close: 0 };
    (window as any).whiteCellStream = result;
    socket.onmessage = () => result.frames++;
    socket.onclose = event => { result.close = event.code; };
  }, { base: backendUrl, seat: await selection(resumedTab) });
  await expect.poll(() => resumedTab.evaluate(() => (window as any).whiteCellStream.frames)).toBeGreaterThan(0);
  await page.getByRole("button", { name: /^Team & players/ }).click();
  await page.locator(".team-panel li").filter({ hasText: "Late referee" }).getByRole("button", { name: "Remove from team" }).click();
  for (const target of [referee, resumedTab]) {
    await expect(target.getByRole("region", { name: "White Cell workspace" })).toHaveCount(0, { timeout: 15_000 });
    await expect(target.getByText("Hold the exercise for referee review.")).toHaveCount(0);
  }
  await expect.poll(() => resumedTab.evaluate(() => (window as any).whiteCellStream.close)).toBe(1008);
  expect((await referee.request.get(`${prefix}/truth?role_id=${captured.role_id}&lease_generation=${captured.lease_generation}`)).status()).toBe(403);
  await page.getByRole("button", { name: "Close team panel" }).click();
  await expect(page.getByRole("region", { name: "White Cell workspace" })).toBeVisible({ timeout: 30_000 });
  await page.getByRole("button", { name: "Truth map", exact: true }).click();
  await page.screenshot({ path: info.outputPath("white-cell-truth.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Team notebook", exact: true }).click();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath("white-cell-phone.png"), fullPage: true });
});

test("White Cell joins a paused game and an approved transfer clears referee access", async ({ page, browser }) => {
  test.setTimeout(90_000);
  await open(page); await page.getByRole("button", { name: /^Command Link Exercise/ }).click();
  await page.getByLabel("Game title").fill("Paused referee entry"); await page.getByRole("button", { name: "Create game", exact: true }).click();
  await page.getByRole("button", { name: /^Exercise Commander/ }).click(); await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Pause scenario", exact: true })).toBeVisible(); await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  const context = await browser.newContext(); contexts.push(context); const referee = await context.newPage();
  await join(referee, "Paused referee", "Paused referee entry"); await referee.locator(".team-card").filter({ hasText: /^White Cell/ }).click();
  await page.getByRole("button", { name: /^Team & players/ }).click();
  await page.locator(".team-request").filter({ hasText: "Paused referee" }).getByRole("button", { name: "Approve", exact: true }).click();
  await page.getByRole("button", { name: "Close team panel" }).click();
  await expect(referee.getByRole("region", { name: "White Cell workspace" })).toBeVisible({ timeout: 30_000 });
  await expect(referee.getByRole("button", { name: "Resume scenario", exact: true })).toBeVisible();
  // A failed or stale referee control must be visible, and must not resume a paused game.
  await referee.route("**/v1/games/*/time-scale", route => route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ code: "control_revision", error: "Game controls changed; refresh and try again" }) }));
  await referee.getByRole("combobox", { name: "Simulation speed", exact: true }).selectOption("2");
  await expect(referee.getByRole("alert")).toContainText("Game controls changed; refresh and try again");
  await referee.unroute("**/v1/games/*/time-scale");
  await referee.getByRole("combobox", { name: "Simulation speed", exact: true }).selectOption("5");
  await expect(referee.getByRole("alert")).toHaveCount(0);
  await expect(referee.getByRole("button", { name: "Resume scenario", exact: true })).toBeVisible();
  await referee.getByRole("button", { name: /^Team & players/ }).click(); referee.once("dialog", dialog => dialog.accept());
  await referee.locator(".team-card").filter({ hasText: /^Blue/ }).click();
  await page.getByRole("button", { name: /^Team & players/ }).click();
  await page.locator(".team-request").filter({ hasText: "Paused referee" }).getByRole("button", { name: "Approve", exact: true }).click();
  await expect(referee.getByRole("region", { name: "White Cell workspace" })).toHaveCount(0, { timeout: 15_000 });
  await referee.getByRole("button", { name: "Close team panel" }).click();
  await referee.getByRole("button", { name: /^Pilot, CAP Alpha 1/ }).click();
  await expect(referee.locator(".globe canvas")).toBeVisible();
  await expect(referee.getByRole("button", { name: "Resume scenario", exact: true })).toHaveCount(0);
});
