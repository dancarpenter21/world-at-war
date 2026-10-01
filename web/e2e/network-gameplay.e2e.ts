import { randomUUID } from "node:crypto";
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

test("plays the command exercise without a catalog and retains a congested radio queue through pause", async ({ page, request }, testInfo) => {
  const exerciseRole = "00000000-0000-0000-0000-000000004e86";
  const secondTarget = "00000000-0000-0000-0000-00000000000c";
  const initialCatalog = await (await request.get(backend.url + "/v1/settings/space-catalog/status")).json();
  expect(initialCatalog.usable).toBe(false);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), playerId);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.goto("/");
  await expect(page.getByRole("button", { name: /^Command Link Exercise/ })).toBeVisible();
  await expect(page.getByText("LOCAL SCENARIO READY", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Game title")).toHaveValue("Command Link Exercise");
  const creation = page.waitForResponse((response) => response.url().endsWith("/v1/games") && response.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const created = await (await creation).json();
  const gameId = created.game.id as string;
  expect(created.game.space_catalog_enabled).toBe(false);
  await expect(page.getByRole("button", { name: /^Exercise National Authority/ })).toBeDisabled();
  const claim = page.waitForResponse((response) => response.url().endsWith(`/roles/${exerciseRole}/claim`));
  await page.getByRole("button", { name: /^Exercise Commander/ }).click();
  const heldRole = await (await claim).json();
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  const movement = page.getByRole("region", { name: "Movement orders" });
  await expect(movement.getByRole("button", { name: "Send movement order", exact: true })).toBeEnabled();
  await expect(movement.getByLabel("Command unit").locator("option")).toHaveCount(2);
  const submit = async (target: string, course: string) => {
    await movement.getByLabel("Command unit").selectOption(target);
    await movement.getByLabel("Course (°)").fill(course);
    await movement.getByLabel("Speed (m/s)").fill("80");
    const response = page.waitForResponse((item) => item.url().endsWith(`/roles/${exerciseRole}/intent`) && item.request().method() === "POST");
    await movement.getByRole("button", { name: "Send movement order", exact: true }).click();
    const completed = await response;
    expect(completed.ok()).toBe(true);
    return { outcome: await completed.json(), body: completed.request().postDataJSON() };
  };
  const first = await submit(targetId, "90");
  const second = await submit(secondTarget, "270");
  const authorization = new URLSearchParams({ player_id: playerId, role_id: exerciseRole });
  const networkUrl = `${backend.url}/v1/games/${gameId}/network?${authorization}`;
  // Wait for packet admission at a simulation boundary, then pause through the
  // API. Queued submission records can precede live channel occupancy.
  const burst = await Promise.all(Array.from({ length: 6 }, async () => {
    const response = await request.post(`${backend.url}/v1/games/${gameId}/roles/${exerciseRole}/intent`, {
      data: { ...first.body, intent: { ...first.body.intent, intent_id: randomUUID() } }
    });
    expect(response.ok()).toBe(true);
    return await response.json() as { message_id: string };
  }));
  await expect.poll(async () => {
    const network = await (await request.get(networkUrl)).json();
    return network.links.reduce((count: number, link: { queued_packets: number }) => count + link.queued_packets, 0);
  }, { timeout: 10_000 }).toBeGreaterThan(0);
  const pause = await request.post(`${backend.url}/v1/games/${gameId}/pause`, { data: { player_id: playerId } });
  expect(pause.ok()).toBe(true);
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  const paused = await (await request.get(networkUrl)).json();
  expect(paused.links.some((link: { queued_packets: number }) => link.queued_packets > 0)).toBe(true);
  expect(paused.links.every((link: { queued_packets: number }) => link.queued_packets <= 4)).toBe(true);
  expect(paused.messages.filter((record: { message: { id: string }; state: string }) =>
    [first.outcome.message_id, second.outcome.message_id, ...burst.map((order) => order.message_id)].includes(record.message.id)).some((record: { state: string }) => record.state !== "delivered")).toBe(true);
  await page.waitForTimeout(1_100);
  const stillPaused = await (await request.get(networkUrl)).json();
  expect(stillPaused).toEqual(paused);
  await page.getByRole("button", { name: "Resume scenario", exact: true }).click();
  const receiptAuthorization = new URLSearchParams({ player_id: playerId, lease_generation: String(heldRole.lease_generation) });
  for (const order of [first, second]) {
    const url = `${backend.url}/v1/games/${gameId}/roles/${exerciseRole}/intents/${order.body.intent.intent_id}?${receiptAuthorization}`;
    await expect.poll(async () => (await (await request.get(url)).json()).state, { timeout: 15_000 }).toBe("executed");
  }
  await expect(movement.getByRole("status")).toContainText("Order executed", { timeout: 10_000 });
  await expect(movement.locator(".movement-current")).toHaveText("Current order: 270° at 80 m/s", { timeout: 10_000 });
  const state = await (await request.get(`${backend.url}/v1/games/${gameId}/state?${authorization}`)).json();
  expect(state.own_units.find((unit: { id: string }) => unit.id === targetId).velocity).toMatchObject({ north_mps: 0, east_mps: 80 });
  expect(state.own_units.find((unit: { id: string }) => unit.id === secondTarget).velocity).toMatchObject({ north_mps: 0, east_mps: -80 });
  expect(state.own_units.find((unit: { id: string }) => unit.id.endsWith("000000000005")).velocity).toMatchObject({ north_mps: 0, east_mps: 0 });
  await page.screenshot({ path: testInfo.outputPath("command-link-exercise-executed.png") });
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});
test("withholds remote sensor knowledge and message contents until radio delivery", async ({ page, request }, testInfo) => {
  const commander = "00000000-0000-0000-0000-000000004e86";
  const sensorRole = "00000000-0000-0000-0000-000000004e87";
  const otherPilot = "00000000-0000-0000-0000-000000004e88";
  const observerPlayer = randomUUID();
  const otherPlayer = randomUUID();
  const enemyId = "00000000-0000-0000-0000-000000000033";
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), playerId);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.goto("/");
  await page.getByRole("button", { name: /^Sensor Relay Exercise/ }).click();
  const creation = page.waitForResponse((response) => response.url().endsWith("/v1/games") && response.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const gameId = (await (await creation).json()).game.id as string;
  await page.getByRole("button", { name: /^Exercise Commander/ }).click();
  for (const [role, player] of [[sensorRole, observerPlayer], [otherPilot, otherPlayer]]) {
    const claim = await request.post(`${backend.url}/v1/games/${gameId}/roles/${role}/claim`, { data: { player_id: player } });
    expect(claim.ok()).toBe(true);
  }
  const authorization = new URLSearchParams({ player_id: playerId, role_id: commander });
  const observerAuthorization = new URLSearchParams({ player_id: observerPlayer, role_id: sensorRole });
  const otherAuthorization = new URLSearchParams({ player_id: otherPlayer, role_id: otherPilot });
  const stateUrl = `${backend.url}/v1/games/${gameId}/state?${authorization}`;
  const networkUrl = `${backend.url}/v1/games/${gameId}/network?${authorization}`;
  const observerNetwork = `${backend.url}/v1/games/${gameId}/network?${observerAuthorization}`;
  const started = page.waitForResponse((response) => response.url().endsWith(`/games/${gameId}/start`));
  const startClick = page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await started;
  await expect.poll(async () => (await (await request.get(observerNetwork)).json()).messages.length, { timeout: 10_000, intervals: [100] }).toBe(1);
  const pause = await request.post(`${backend.url}/v1/games/${gameId}/pause`, { data: { player_id: playerId } });
  expect(pause.ok()).toBe(true);
  await startClick;
  const source = (await (await request.get(observerNetwork)).json()).messages[0];
  expect(source.state).toBe("in_transit");
  expect(source.message.profile_id).toBe("public-safe.jseries.track-report.v1");
  expect(source.message.fields.track.track_id).not.toBe(enemyId);
  expect(JSON.stringify(source.message)).not.toContain(enemyId);
  const initial = await (await request.get(stateUrl)).json();
  expect(initial.tracks).toEqual([]);
  expect(initial.own_units.every((unit: { id: string }) => unit.id !== enemyId)).toBe(true);
  expect((await (await request.get(networkUrl)).json()).messages).toEqual([]);
  expect((await (await request.get(`${backend.url}/v1/games/${gameId}/network/events?${authorization}`)).json()).events).toEqual([]);
  expect((await request.get(`${backend.url}/v1/games/${gameId}/network/messages/${source.message.id}?${authorization}`)).status()).toBe(404);
  expect((await (await request.get(`${backend.url}/v1/games/${gameId}/state?${observerAuthorization}`)).json()).tracks).toHaveLength(1);
  expect((await (await request.get(`${backend.url}/v1/games/${gameId}/state?${otherAuthorization}`)).json()).tracks).toEqual([]);
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(page.getByText("No reports received.", { exact: true })).toBeVisible();
  const streamFrames: { projection: { messages: { state: string; message: { id: string } }[] } }[] = [];
  const stream = new WebSocket(`${backend.url.replace("http:", "ws:")}/v1/games/${gameId}/network/stream?${authorization}`);
  stream.addEventListener("message", (event) => streamFrames.push(JSON.parse(String(event.data))));
  try {
    await expect.poll(() => streamFrames.length, { timeout: 5_000 }).toBeGreaterThan(0);
    expect(streamFrames[0].projection.messages).toEqual([]);
    await page.getByRole("button", { name: "Resume scenario", exact: true }).click();
    await expect.poll(async () => (await (await request.get(stateUrl)).json()).tracks.length, { timeout: 10_000, intervals: [100] }).toBe(1);
    expect((await request.post(`${backend.url}/v1/games/${gameId}/pause`, { data: { player_id: playerId } })).ok()).toBe(true);
    const received = await (await request.get(stateUrl)).json();
    const track = received.tracks[0];
    expect(track.observed_tick).toBe(source.message.fields.track.observed_tick);
    expect(track.received_tick).toBeGreaterThan(track.observed_tick);
    expect(track.position).toEqual(source.message.fields.track.position);
    expect(track.track_id).not.toBe(source.message.fields.track.track_id);
    expect(track.track_id).not.toBe(enemyId);
    const delivered = (await (await request.get(networkUrl)).json()).messages[0];
    expect(delivered.state).toBe("delivered");
    const events = (await (await request.get(`${backend.url}/v1/games/${gameId}/network/events?${authorization}`)).json()).events;
    expect(events.map((event: { state: string }) => event.state)).toEqual(["delivered"]);
    expect((await (await request.get(`${backend.url}/v1/games/${gameId}/state?${otherAuthorization}`)).json()).tracks).toEqual([]);
    await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
    const report = page.locator(".track");
    await expect(report).toHaveCount(1);
    await expect(report).toContainText("Uncertain Red contact");
    await expect(report).toContainText("Last known position");
    await expect(report).toContainText(`${track.received_tick - track.observed_tick}s delivery delay`);
    await page.screenshot({ path: testInfo.outputPath("sensor-report-delivered.png") });
    await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
    await expect.poll(() => streamFrames.some((frame) => frame.projection.messages.some((record) => record.message.id === source.message.id)), { timeout: 5_000 }).toBe(true);
    expect(streamFrames.flatMap((frame) => frame.projection.messages).every((record) => record.state === "delivered")).toBe(true);
    expect(errors).toEqual([]);
  } finally {
    stream.close();
  }
});
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
  const heldRole = await (await claim).json() as { lease_generation: number; location_unit_id: string };
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
  const visibleNetwork = await (await request.get(`${backend.url}/v1/games/${gameId}/network?${authorization}`)).json() as {
    nodes: { id: string; name: string }[]; links: { from_entity_id: string; to_entity_id: string }[];
  };
  const issuingTerminal = visibleNetwork.nodes.find((node) => node.id === heldRole.location_unit_id)!;
  const incidentLinks = visibleNetwork.links.filter((link) => link.from_entity_id === issuingTerminal.id || link.to_entity_id === issuingTerminal.id);
  await expect(page.locator(".network-focus")).toHaveText(`Connections of ${issuingTerminal.name}`);
  await expect(page.locator(".react-flow__edge")).toHaveCount(incidentLinks.length);
  expect(incidentLinks.length).toBeLessThan(250);
  expect(await page.locator("*").count()).toBeLessThan(2_000);
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

test("rejoins a paused game through the players held role without resuming or claiming another players slot", async ({ page, request }) => {
  const commanderRole = "00000000-0000-0000-0000-000000004e86";
  const pilotRole = "00000000-0000-0000-0000-000000004e87";
  const otherPlayer = "00000000-0000-4000-8000-000000008099";
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), playerId);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.goto("/");
  await page.getByRole("button", { name: /^Command Link Exercise/ }).click();
  await page.getByLabel("Game title").fill("Held role rejoin");
  const creation = page.waitForResponse((response) => response.url().endsWith("/v1/games") && response.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const gameId = (await (await creation).json()).game.id as string;
  const claim = page.waitForResponse((response) => response.url().endsWith(`/roles/${commanderRole}/claim`));
  await page.getByRole("button", { name: /^Exercise Commander/ }).click();
  const firstLease = (await (await claim).json()).lease_generation as number;
  const pilotClaim = await request.post(`${backend.url}/v1/games/${gameId}/roles/${pilotRole}/claim`, { data: { player_id: otherPlayer } });
  expect(pilotClaim.ok()).toBe(true);
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect(page.locator(".globe canvas")).toBeVisible();
  await page.getByRole("button", { name: "Pause scenario", exact: true }).click();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  const authorization = new URLSearchParams({ player_id: playerId, role_id: commanderRole });
  const pausedState = await (await request.get(`${backend.url}/v1/games/${gameId}/state?${authorization}`)).json() as { tick: number };
  const pausedTick = `TICK ${pausedState.tick}`;
  await expect(page.locator("header .tick")).toHaveText(pausedTick);
  const anonymous = await (await request.get(`${backend.url}/v1/games/${gameId}/roles`)).json() as { id: string; held_by_you: boolean }[];
  expect(anonymous.every((role) => !role.held_by_you)).toBe(true);
  expect(JSON.stringify(anonymous)).not.toContain(otherPlayer);
  const foreignClaim = await request.post(`${backend.url}/v1/games/${gameId}/roles/${commanderRole}/claim`, { data: { player_id: otherPlayer } });
  expect(foreignClaim.status()).toBe(409);
  await page.getByRole("button", { name: "Leave scenario", exact: true }).click();
  await expect(page.getByRole("button", { name: "Create game", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Join game", exact: true }).click();
  await page.getByRole("button", { name: /Held role rejoin.*paused/ }).click();
  const owned = page.getByRole("button", { name: /^Exercise Commander/ });
  await expect(owned).toBeEnabled();
  await expect(owned).toContainText("your role");
  await expect(page.getByRole("button", { name: /^Pilot, CAP Alpha 1/ })).toBeDisabled();
  const reclaim = page.waitForResponse((response) => response.url().endsWith(`/roles/${commanderRole}/claim`));
  await owned.click();
  const secondLease = (await (await reclaim).json()).lease_generation as number;
  expect(secondLease).toBeGreaterThan(firstLease);
  const staleOrder = await request.post(`${backend.url}/v1/games/${gameId}/roles/${commanderRole}/intent`, { data: {
    player_id: playerId, lease_generation: firstLease,
    intent: { intent_id: randomUUID(), issuer_role: commanderRole, target: targetId,
      kind: { Move: { north_mps: 0, east_mps: 0 } }, requested_tick: 0 }
  } });
  expect(staleOrder.status()).toBe(403);
  expect((await staleOrder.json()).code).toBe("invalid_role_lease");
  await expect(page.locator(".globe canvas")).toBeVisible();
  await expect(page.getByText("Scenario paused", { exact: true })).toBeVisible();
  await expect(page.locator("header .tick")).toHaveText(pausedTick!);
  await expect(page.getByRole("button", { name: "Send movement order", exact: true })).toBeDisabled();
  const summary = await (await request.get(backend.url + "/v1/games")).json() as { id: string; status: string }[];
  expect(summary.find((game) => game.id === gameId)!.status).toBe("paused");
  expect(errors).toEqual([]);
});

test("completes combat training after a radio-delivered engagement and safely retries a lost submission response", async ({ page, request }, testInfo) => {
  const commander = "00000000-0000-0000-0000-000000004e86";
  const enemyId = "00000000-0000-0000-0000-000000000033";
  const errors: string[] = [];
  const submissions: unknown[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), playerId);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.route(`**/roles/${commander}/intent`, async (route) => {
    submissions.push(route.request().postDataJSON());
    if (submissions.length === 1) {
      const accepted = await route.fetch();
      expect(accepted.ok()).toBe(true);
      await route.abort("failed");
    } else { await route.continue(); }
  });
  await page.goto("/");
  await page.getByRole("button", { name: /^Combat Training Exercise/ }).click();
  await page.getByLabel("Game title").fill("Combat retry exercise");
  const creation = page.waitForResponse((response) => response.url().endsWith("/v1/games") && response.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const gameId = (await (await creation).json()).game.id as string;
  await page.getByRole("button", { name: /^Exercise Commander/ }).click();
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  const mission = page.getByRole("region", { name: "Training mission" });
  const orders = page.getByRole("region", { name: "Engagement orders" });
  await expect(mission).toContainText("Mission active");
  await expect(orders.getByRole("button", { name: "Send engagement order", exact: true })).toBeEnabled({ timeout: 15_000 });
  await expect(orders).toContainText("Training ammunition: 2");
  await orders.getByRole("button", { name: "Send engagement order", exact: true }).click();
  await expect(orders.getByRole("status")).toContainText("server response was lost");
  await orders.getByRole("button", { name: "Retry engagement", exact: true }).click();
  expect(submissions).toHaveLength(2);
  expect(submissions[1]).toEqual(submissions[0]);
  await expect(orders.getByRole("status")).toContainText("Weapon launched", { timeout: 20_000 });
  await expect(orders).toContainText("Training ammunition: 1");
  await expect(mission).toContainText("Mission complete", { timeout: 15_000 });
  await expect(page.getByRole("button", { name: "Resume scenario", exact: true })).toBeDisabled();
  await expect(orders.getByRole("button", { name: "Send engagement order", exact: true })).toBeDisabled();
  const authorization = new URLSearchParams({ player_id: playerId, role_id: commander });
  const stateUrl = `${backend.url}/v1/games/${gameId}/state?${authorization}`;
  const state = await (await request.get(stateUrl)).json();
  expect(state.combat.mission.status).toBe("succeeded");
  expect(state.combat.local_impacts).toEqual([]);
  expect(JSON.stringify(state)).not.toContain(enemyId);
  const network = await (await request.get(`${backend.url}/v1/games/${gameId}/network?${authorization}`)).json();
  const commands = network.messages.filter((record: { message: { profile_id: string } }) => record.message.profile_id === "public-safe.jseries.engage-order.v1");
  expect(commands).toHaveLength(1);
  expect(commands[0].state).toBe("delivered");
  expect(commands[0].message.fields.aim_point).toEqual(state.tracks[0].position);
  const resume = await request.post(`${backend.url}/v1/games/${gameId}/start`, { data: { player_id: playerId } });
  expect(resume.status()).toBe(409);
  expect((await resume.json()).code).toBe("mission_complete");
  await page.waitForTimeout(1_200);
  expect((await (await request.get(stateUrl)).json()).tick).toBe(state.tick);
  await mission.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("combat-training-complete.png") });
  expect(errors).toEqual([]);
});

test("a pilot requests firing authority, respects denial, and launches only after commander approval arrives", async ({ page, browser, request }, testInfo) => {
  const commander = "00000000-0000-0000-0000-000000004e86";
  const pilotRole = "00000000-0000-0000-0000-000000004e87";
  const pilotPlayer = "00000000-0000-4000-8000-000000009001";
  await page.addInitScript((id) => localStorage.setItem("world-at-war-player", id), playerId);
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
  await page.goto("/");
  await page.getByRole("button", { name: /^Combat Training Exercise/ }).click();
  await page.getByLabel("Game title").fill("Combat authority exercise");
  const creation = page.waitForResponse((response) => response.url().endsWith("/v1/games") && response.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const gameId = (await (await creation).json()).game.id as string;
  await page.getByRole("button", { name: /^Exercise Commander/ }).click();
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  const pilotContext = await browser.newContext({ baseURL: page.url() });
  try {
    const pilot = await pilotContext.newPage();
    await pilot.addInitScript((id) => localStorage.setItem("world-at-war-player", id), pilotPlayer);
    await pilot.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, (route) => route.abort());
    await pilot.goto("/");
    await pilot.getByRole("button", { name: "Join game", exact: true }).click();
    await pilot.getByRole("button", { name: /Combat authority exercise.*running/ }).click();
    await pilot.getByRole("button", { name: /^Pilot, CAP Alpha 1/ }).click();
    const orders = pilot.getByRole("region", { name: "Engagement orders" });
    const authorization = new URLSearchParams({ player_id: pilotPlayer, role_id: pilotRole });
    const ammunition = async () => (await (await request.get(`${backend.url}/v1/games/${gameId}/state?${authorization}`)).json()).own_units.find((unit: { id: string }) => unit.id === targetId).weapon.ammunition;
    await expect(orders.getByRole("button", { name: "Send engagement order", exact: true })).toBeEnabled();
    const submitted = pilot.waitForResponse((response) => response.url().endsWith(`/roles/${pilotRole}/intent`) && response.request().method() === "POST");
    await orders.getByRole("button", { name: "Send engagement order", exact: true }).click();
    const pendingRequest = await (await submitted).json() as { request_id: string; message_id: string };
    const commanderAuthorization = new URLSearchParams({ player_id: playerId, role_id: commander });
    const beforeDelivery = await (await request.get(`${backend.url}/v1/games/${gameId}/authority/requests?${commanderAuthorization}`)).json() as { id: string }[];
    expect(beforeDelivery.some((item) => item.id === pendingRequest.request_id)).toBe(false);
    const withheld = await request.get(`${backend.url}/v1/games/${gameId}/network/messages/${pendingRequest.message_id}?${commanderAuthorization}`);
    expect(withheld.status()).toBe(404);
    await page.getByRole("button", { name: /^Authorities/ }).click();
    await page.getByRole("button", { name: "requests", exact: true }).click();
    await expect(page.getByRole("button", { name: "Deny", exact: true })).toBeVisible({ timeout: 15_000 });
    expect(await ammunition()).toBe(2);
    await expect(page.locator(".request-card")).toContainText("Engage reported Red contact");
    await page.screenshot({ path: testInfo.outputPath("combat-firing-authority.png") });
    await page.getByRole("button", { name: "Deny", exact: true }).click();
    await expect(orders.getByRole("status")).toContainText("Firing authority denied", { timeout: 10_000 });
    expect(await ammunition()).toBe(2);
    const deniedCard = page.locator(`[data-request-id="${pendingRequest.request_id}"]`);
    await expect(deniedCard.locator(".request-status")).toHaveText("denied");
    await expect(deniedCard.getByRole("button", { name: "Approve", exact: true })).toHaveCount(0);
    const resubmitted = pilot.waitForResponse((response) => response.url().endsWith(`/roles/${pilotRole}/intent`) && response.request().method() === "POST");
    await orders.getByRole("button", { name: "Send engagement order", exact: true }).click();
    const secondRequest = await (await resubmitted).json() as { request_id: string };
    expect(secondRequest.request_id).not.toBe(pendingRequest.request_id);
    const newCard = page.locator(`[data-request-id="${secondRequest.request_id}"]`);
    await expect(newCard.getByRole("button", { name: "Approve", exact: true })).toBeVisible({ timeout: 15_000 });
    expect(await ammunition()).toBe(2);
    await newCard.getByRole("button", { name: "Approve", exact: true }).click();
    await expect(orders.getByRole("status")).toContainText("Weapon launched", { timeout: 20_000 });
    await expect(pilot.getByRole("region", { name: "Training mission" })).toContainText("Mission complete", { timeout: 15_000 });
    await expect(orders).toContainText("Weapon impact: hit");
    expect(await ammunition()).toBe(1);
    const commanderState = await (await request.get(`${backend.url}/v1/games/${gameId}/state?${new URLSearchParams({ player_id: playerId, role_id: commander })}`)).json();
    expect(commanderState.combat.local_impacts).toEqual([]);
  } finally { await pilotContext.close(); }
});
