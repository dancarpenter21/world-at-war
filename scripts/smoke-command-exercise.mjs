import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

export async function smokeCommandExercise(baseUrl, { checkWeb = true } = {}) {
  let cookie = "";
  let csrf = "";
  const origin = process.env.SMOKE_ORIGIN ?? new URL(baseUrl).origin;
  const request = async (resource, body) => {
    const response = await fetch(new URL(resource, baseUrl), {
      method: body === undefined ? "GET" : "POST",
      headers: { "content-type": "application/json", origin, cookie, "x-csrf-token": csrf },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: AbortSignal.timeout(10_000)
    });
    assert.equal(response.ok, true, `${resource} returned HTTP ${response.status}`);
    const setCookie = response.headers.get("set-cookie");
    if (setCookie) cookie = setCookie.split(";")[0];
    return await response.json();
  };
  assert.equal((await request("/health")).status, "ok");
  if (checkWeb) {
    const response = await fetch(new URL("/", baseUrl), { signal: AbortSignal.timeout(10_000) });
    assert.equal(response.ok, true, "web index is unavailable");
    const html = await response.text();
    assert.match(html, /world at war/i);
    const script = html.match(/<script[^>]*src="([^"]+)"/);
    assert.ok(script, "production index has no script asset");
    const asset = await fetch(new URL(script[1], baseUrl), { signal: AbortSignal.timeout(10_000) });
    assert.equal(asset.ok, true, "production script asset is unavailable");
    assert.match(asset.headers.get("content-type") ?? "", /javascript/);
    await asset.arrayBuffer();
  }
  const scenarios = await request("/v1/scenarios");
  const scenario = scenarios.find((item) => item.id === "command-link-exercise.v1");
  assert.ok(scenario, "command exercise was not embedded in the image");
  assert.equal(scenario.requires_space_catalog, false);
  const session = await request("/v1/auth/guest", { display_name: "Production smoke" });
  csrf = session.csrf_token;
  const { game } = await request("/v1/games", {
    scenario_id: scenario.id, title: "Production smoke exercise"
  });
  assert.equal(game.space_catalog_enabled, false);
  const prefix = `/v1/games/${game.id}`;
  const roles = await request(prefix + "/roles");
  const commander = roles.find((role) => role.name === "Exercise Commander");
  assert.ok(commander?.claimable, "exercise commander is not claimable");
  const role = await request(`${prefix}/roles/${commander.id}/claim`, {});
  assert.equal(role.command_units.length, 2);
  assert.equal((await request(prefix + "/start", {})).status, "running");
  const body = {
    lease_generation: role.lease_generation,
    intent: {
      intent_id: randomUUID(), issuer_role: role.id, target: role.command_units[0],
      kind: { Move: { north_mps: 0, east_mps: 60 } }, requested_tick: 0
    }
  };
  const orderUrl = `${prefix}/roles/${role.id}/intent`;
  const submission = await request(orderUrl, body);
  assert.equal(submission.status, "queued");
  assert.deepEqual(await request(orderUrl, body), submission, "retry produced another submission");
  const lease = new URLSearchParams({ lease_generation: String(role.lease_generation) });
  const receiptUrl = `${prefix}/roles/${role.id}/intents/${body.intent.intent_id}?${lease}`;
  let receipt;
  const deadline = Date.now() + 20_000;
  do {
    receipt = await request(receiptUrl);
    if (receipt.state === "executed") break;
    assert.ok(!["dropped", "expired", "rejected", "denied"].includes(receipt.state), `order ended as ${receipt.state}`);
    await new Promise((resolve) => setTimeout(resolve, 250));
  } while (Date.now() < deadline);
  assert.equal(receipt.state, "executed", "command did not reach the simulation executor");
  const authorization = new URLSearchParams({ role_id: role.id, lease_generation: String(role.lease_generation) });
  const stateUrl = `${prefix}/state?${authorization}`;
  const projection = await request(stateUrl);
  const unit = projection.own_units.find((item) => item.id === body.intent.target);
  assert.equal(unit.velocity.north_mps, 0);
  assert.equal(unit.velocity.east_mps, 60);
  const events = await request(`${prefix}/network/events?${authorization}`);
  assert.deepEqual(events.events.filter((event) => event.message.id === submission.message_id).map((event) => event.state),
    ["queued", "in_transit", "delivered"], "network lifecycle was not recorded exactly once");
  assert.equal((await request(prefix + "/pause", {})).status, "paused");
  const paused = await request(stateUrl);
  await new Promise((resolve) => setTimeout(resolve, 1_100));
  assert.deepEqual(await request(stateUrl), paused, "paused simulation state continued changing");
  console.log(`Smoke passed: ${checkWeb ? "static assets, " : ""}catalog-free game, retry, packet delivery, execution, and pause.`);
  return game.id;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: {
    "base-url": { type: "string", default: "http://127.0.0.1:8080" },
    "skip-web": { type: "boolean", default: false }
  } });
  await smokeCommandExercise(values["base-url"], { checkWeb: !values["skip-web"] });
}