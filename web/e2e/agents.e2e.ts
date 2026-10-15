import { expect, test } from "@playwright/test";
import { createServer, type Server } from "node:http";
import { spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import path from "node:path";
import { backendUrl, repoRoot, startBackend, type Backend } from "./support/server";
let backend: Backend;
let provider: Server | undefined;
let runner: ChildProcess | undefined;
test.beforeEach(async () => { backend = await startBackend(); });
test.afterEach(async ({}, info) => {
  if (runner && runner.exitCode === null && runner.signalCode === null) { const exited = once(runner, "exit"); runner.kill("SIGTERM"); await exited; }
  runner = undefined;
  if (provider) { provider.closeAllConnections(); await new Promise<void>(resolve => provider!.close(() => resolve())); provider = undefined; }
  if (info.status !== info.expectedStatus) await info.attach("backend.log", { body: backend.output(), contentType: "text/plain" });
  await backend.stop();
});
for (const phone of [false, true]) test(`external OpenRouter agent, fallback and token rotation on ${phone ? "phone" : "desktop"}`, async ({ page }, info) => {
  test.setTimeout(90000);
  if (phone) await page.setViewportSize({ width: 390, height: 844 });
  await page.route(/https:\/\/[^/]*tile\.openstreetmap\.org\//, route => route.abort());
  await page.goto("/");
  await page.getByRole("button", { name: /^AI Defense Exercise/ }).click();
  await page.getByLabel("Join as", { exact: true }).selectOption("white_cell");
  const created = page.waitForResponse(r => r.url().endsWith("/v1/games") && r.request().method() === "POST");
  await page.getByRole("button", { name: "Create game", exact: true }).click();
  const gameId = (await (await created).json()).game.id as string;
  const prefix = `${backendUrl}/v1/games/${gameId}`;
  await expect(page.getByRole("region", { name: "White Cell workspace" })).toBeVisible({ timeout: 30000 });
  await page.getByRole("button", { name: /^Team & players/ }).click();
  const panel = page.getByRole("region", { name: "AI controllers" });
  await expect(panel).toBeVisible();
  await panel.getByLabel("Agent response timeout (seconds)").fill("3");
  const role = "00000000-0000-0000-0000-00000000526d"; // Blue aircraft: mixed human/agent sides.
  const assigned = page.waitForResponse(r => r.url().endsWith(`/agents/${role}`) && r.request().method() === "POST");
  await panel.getByLabel("Controller for Blue Aircraft Operator").selectOption("external");
  const token = (await (await assigned).json()).token as string;
  await expect(panel.getByText(token, { exact: true })).toBeVisible();
  await panel.getByRole("button", { name: "Hide token", exact: true }).click();
  await page.getByRole("button", { name: "Close team panel", exact: true }).click();
  const requests: { model: string; messages: { content: string }[] }[] = [];
  let releaseInference!: () => void;
  const inferenceGate = new Promise<void>(resolve => { releaseInference = resolve; });
  provider = createServer(async (req, res) => {
    let body = ""; for await (const chunk of req) body += String(chunk);
    expect(req.url).toBe("/api/v1/chat/completions"); expect(req.headers.authorization).toBe("Bearer fixture-provider-key");
    requests.push(JSON.parse(body));
    await inferenceGate;
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ model: "fixture/model:free", choices: [{ finish_reason: "stop", message: { content: JSON.stringify({ actions: [{ type: "order", unit_id: "00000000-0000-0000-0000-000000000033", kind: { Move: { north_mps: 0, east_mps: 35 } } }], explanation: "Move based on the assigned role observation." }) } }] }));
  });
  provider.listen(0, "127.0.0.1"); await once(provider, "listening");
  const address = provider.address(); if (!address || typeof address === "string") throw new Error("Missing provider port");
  runner = spawn(process.execPath, [path.join(repoRoot, "web/scripts/agent-runner.mjs")], {
    cwd: repoRoot, env: { ...process.env, OPENROUTER_API_KEY: "fixture-provider-key", OPENROUTER_BASE_URL: `http://127.0.0.1:${address.port}/api/v1`, OPENROUTER_MODEL: "openrouter/free", OPENROUTER_TIMEOUT_MS: "2000", OPENROUTER_MIN_INTERVAL_MS: "1000", WAW_SERVER_URL: backendUrl, WAW_AGENT_TOKEN: token, WAW_GAME_ID: gameId }, stdio: "ignore",
  });
  await page.getByRole("button", { name: "Start scenario", exact: true }).click();
  await expect.poll(() => requests.length).toBeGreaterThan(0);
  expect(requests[0].model).toBe("openrouter/free");
  const observation = JSON.parse(requests[0].messages[1].content);
  expect(observation.role_id).toBe(role); expect(observation.side).toBe("Blue");
  expect(observation.projection.tracks).toHaveLength(0);
  expect(observation.projection.own_units.every((u: { name: string }) => !u.name.startsWith("Red"))).toBe(true);
  // Keep inference outstanding and prove the authoritative clock still advances.
  try {
    const elapsed = async () => {
      const games = await (await page.request.get(`${backendUrl}/v1/games`)).json();
      return games.find((g: { id: string }) => g.id === gameId).elapsed_seconds as number;
    };
    const before = await elapsed();
    await expect.poll(elapsed, { timeout: 1500, intervals: [100, 100, 100] }).toBeGreaterThan(before + 0.15);
  } finally { releaseInference(); }

  await expect.poll(async () => {
    const view = await (await page.request.get(`${prefix}/agents`)).json();
    return view.history.some((e: { kind: string; details: { model?: string } }) => e.kind === "decision" && e.details.model === "fixture/model:free");
  }).toBe(true);
  const exited = once(runner, "exit"); runner.kill("SIGTERM"); await exited; runner = undefined;
  await page.getByRole("button", { name: /^Team & players/ }).click();
  await expect(panel.getByText(/Blue Aircraft Operator.*built-in fallback/)).toBeVisible({ timeout: 20000 });
  await panel.getByRole("button", { name: "Rotate token", exact: true }).click();
  await expect(panel.getByText(/Connection token for role/)).toBeVisible();
  expect((await page.request.get(`${prefix}/agent/work`, { headers: { Authorization: `Bearer ${token}` } })).status()).toBe(401);
  await panel.getByRole("button", { name: "Hide token", exact: true }).click();
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath(`agent-controllers-${phone ? "phone" : "desktop"}.png`), fullPage: true });
});
