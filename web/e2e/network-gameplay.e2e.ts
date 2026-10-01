import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { startGameBackend } from "./fixtures/game-backend";

const playerId = "00000000-0000-4000-8000-000000008001";
const roleId = "00000000-0000-0000-0000-00000000006a";
const targetId = "00000000-0000-0000-0000-00000000000b";
let backend: Awaited<ReturnType<typeof startGameBackend>>;

test.beforeAll(async () => { backend = await startGameBackend(); });
test.afterAll(async () => { if (backend) await backend.close(); });

test("runs a real game, persists networked command delivery, and retains the map through host pause", async ({ page, request }, testInfo) => {
  const started = Date.now();
  const timings: { stage: string; elapsed_ms: number; dom_nodes: number; communications_rows: number }[] = [];
  const mark = async (stage: string) => timings.push({ stage, elapsed_ms: Date.now() - started,
    ...await page.evaluate(() => ({ dom_nodes: document.querySelectorAll("*").length, communications_rows: document.querySelectorAll(".communication-status").length })) });
  const connected = await request.post(backend.url + "/v1/admin/space-track/connect", {
    data: { username: "fixture-user", password: "fixture-password", remember: false }
  });
  expect(connected.ok()).toBe(true);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), playerId);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.goto("/");
  await expect(page.locator(".catalog-tab-status.ready")).toBeVisible();
  await mark("catalog-ready");
  await page.getByRole("button", { name: /^Global Crisis/ }).click();
  const creation = page.waitForResponse((response) => response.url().endsWith("/v1/games") && response.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const created = await (await creation).json() as { game: { id: string } };
  const gameId = created.game.id;
  const claim = page.waitForResponse((response) => response.url().endsWith(`/roles/${roleId}/claim`));
  await page.getByRole("button", { name: /^Joint Force Air Component Commander/ }).click();
  const heldRole = await (await claim).json() as { lease_generation: number };
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  const canvas = page.locator(".globe canvas");
  await expect(canvas).toBeVisible();
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeEnabled();
  await canvas.evaluate((element) => element.setAttribute("data-retained", "yes"));
  await expect(page.locator(".communication-status")).toHaveCount(8);
  await mark("map-ready");

  const orderBody = {
    player_id: playerId, lease_generation: heldRole.lease_generation,
    intent: {
      intent_id: "00000000-0000-4000-8000-000000008002", issuer_role: roleId, target: targetId,
      kind: { Move: { north_mps: 10, east_mps: 0 } }, requested_tick: 0
    }
  };
  const orderUrl = `${backend.url}/v1/games/${gameId}/roles/${roleId}/intent`;
  const intent = await request.post(orderUrl, { data: orderBody });
  expect(intent.ok()).toBe(true);
  const submission = await intent.json() as { status: string; message_id: string };
  expect(submission.status).toBe("queued");
  const replay = await request.post(orderUrl, { data: orderBody });
  expect(replay.ok()).toBe(true);
  expect(await replay.json()).toEqual(submission);
  const conflict = await request.post(orderUrl, { data: { ...orderBody, intent: {
    ...orderBody.intent, kind: { Move: { north_mps: 0, east_mps: 10 } }
  } } });
  expect(conflict.status()).toBe(409);
  expect((await conflict.json()).code).toBe("intent_conflict");
  const receiptAuthorization = new URLSearchParams({ player_id: playerId, lease_generation: String(heldRole.lease_generation) });
  const receiptUrl = `${backend.url}/v1/games/${gameId}/roles/${roleId}/intents/${orderBody.intent.intent_id}?${receiptAuthorization}`;
  await expect.poll(async () => (await (await request.get(receiptUrl)).json()).state, { timeout: 10_000 }).toBe("executed");
  const receipt = await (await request.get(receiptUrl)).json();
  expect(receipt.message_id).toBe(submission.message_id);
  expect(receipt.executed_tick).toBeGreaterThanOrEqual(receipt.submitted_tick);
  expect(receipt.intent).toEqual(orderBody.intent);
  const authorization = new URLSearchParams({ player_id: playerId, role_id: roleId });
  const messageUrl = `${backend.url}/v1/games/${gameId}/network/messages/${submission.message_id}?${authorization}`;
  await expect.poll(async () => (await (await request.get(messageUrl)).json()).state, { timeout: 10_000 }).toBe("delivered");
  const history = await (await request.get(`${backend.url}/v1/games/${gameId}/network/events?${authorization}`)).json() as {
    events: { sequence: number; state: string; message: { id: string }; started_at_ns: number | null; delivered_at_ns: number | null }[]
  };
  const transitions = history.events.filter((record) => record.message.id === submission.message_id);
  expect(transitions.map((record) => record.state)).toEqual(["queued", "in_transit", "delivered"]);
  expect(transitions[0].started_at_ns).toBeNull();
  expect(transitions[1].started_at_ns).not.toBeNull();
  expect(transitions[2].delivered_at_ns).toBeGreaterThan(transitions[1].started_at_ns!);
  const eventsFile = path.join(backend.runDirectory, "var/network-events", `${gameId}.network-events.jsonl`);
  const persisted = (await readFile(eventsFile, "utf8")).trim().split("\n").map((line) => JSON.parse(line) as { state?: string; message?: { id: string } });
  expect(persisted.filter((record) => record.message?.id === submission.message_id).map((record) => record.state))
    .toEqual(["queued", "in_transit", "delivered"]);

  await mark("command-delivered");
  const outsider = await request.get(`${backend.url}/v1/games/${gameId}/network?player_id=00000000-0000-4000-8000-000000008099&role_id=${roleId}`);
  expect(outsider.status()).toBe(403);
  const outsiderReceipt = await request.get(receiptUrl.replace(playerId, "00000000-0000-4000-8000-000000008099"));
  expect(outsiderReceipt.status()).toBe(403);
  await page.getByRole("button", { name: "Network", exact: true }).click();
  await expect(page.getByRole("region", { name: "C2 network workspace", exact: true })).toBeVisible();
  await mark("network-open");
  await page.getByRole("tab", { name: /^Messages/ }).click();
  await expect(page.getByRole("button", { name: /^Inspect message: move order/ })).toBeVisible({ timeout: 15_000 });
  await page.getByRole("button", { name: /^Inspect message: move order/ }).click();
  const details = page.getByRole("region", { name: "Message details" });
  await expect(details.locator(".network-state")).toHaveText("delivered");
  await expect(details.getByText("Queue wait", { exact: true })).toBeVisible();
  await expect(details.getByText("Network transit", { exact: true })).toBeVisible();
  await mark("message-inspected");
  await page.screenshot({ path: testInfo.outputPath("real-network-command-delivered.png") });
  await page.getByRole("button", { name: "Back to map", exact: true }).click();
  const movement = page.getByRole("region", { name: "Movement orders" });
  await movement.getByLabel("Command unit").selectOption(targetId);
  await movement.getByLabel("Course (°)").fill("90");
  await movement.getByLabel("Speed (m/s)").fill("80");
  await movement.getByRole("button", { name: "Send movement order", exact: true }).click();
  await expect(movement.getByRole("status")).toContainText("Order executed", { timeout: 10_000 });
  const commanded = await (await request.get(`${backend.url}/v1/games/${gameId}/state?${authorization}`)).json();
  expect(commanded.own_units.find((unit: { id: string }) => unit.id === targetId).velocity).toMatchObject({ north_mps: 0, east_mps: 80 });
  await page.screenshot({ path: testInfo.outputPath("real-movement-order-executed.png") });
  await movement.getByRole("button", { name: "Stop unit", exact: true }).click();
  await expect(movement.getByRole("status")).toContainText("Order executed", { timeout: 10_000 });
  await expect(movement.getByRole("status")).toContainText("Stopped");
  const stopped = await (await request.get(`${backend.url}/v1/games/${gameId}/state?${authorization}`)).json();
  expect(stopped.own_units.find((unit: { id: string }) => unit.id === targetId).velocity).toMatchObject({ north_mps: 0, east_mps: 0 });
  await mark("movement-controls-executed");
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(canvas).toHaveAttribute("data-retained", "yes");
  const pausedState = await (await request.get(`${backend.url}/v1/games/${gameId}/state?${authorization}`)).json() as { tick: number };
  const pausedTick = `TICK ${pausedState.tick}`;
  await expect(page.locator("header .tick")).toHaveText(pausedTick);
  await page.waitForTimeout(1_200);
  await expect(page.locator("header .tick")).toHaveText(pausedTick!);
  await page.getByRole("button", { name: "Resume scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Turn north", exact: true })).toBeEnabled();
  await expect(canvas).toHaveAttribute("data-retained", "yes");
  await mark("resumed");
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
  await mark("left-game");
  await testInfo.attach("gameplay-performance", { body: JSON.stringify(timings, null, 2), contentType: "application/json" });
  console.log("gameplay performance", JSON.stringify(timings));
});
