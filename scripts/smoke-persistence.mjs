import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: {
  "base-url": { type: "string", default: "http://127.0.0.1:8080" },
  prepare: { type: "string" }, verify: { type: "string" }
} });
assert.equal(Boolean(values.prepare) !== Boolean(values.verify), true, "supply --prepare FILE or --verify FILE");
const base = values["base-url"];
const origin = process.env.SMOKE_ORIGIN ?? new URL(base).origin;
let saved = values.verify ? JSON.parse(await readFile(values.verify, "utf8")) : { cookie: "", csrf: "" };
async function request(resource, body) {
  const response = await fetch(new URL(resource, base), {
    method: body === undefined ? "GET" : "POST",
    headers: { origin, cookie: saved.cookie, "x-csrf-token": saved.csrf, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(10_000)
  });
  assert.equal(response.ok, true, `${resource} returned HTTP ${response.status}`);
  const cookie = response.headers.get("set-cookie");
  if (cookie) saved.cookie = cookie.split(";")[0];
  return response.json();
}
if (values.prepare) {
  const session = await request("/v1/auth/guest", { display_name: "Recovery smoke" });
  saved.csrf = session.csrf_token;
  saved.player = session.player_id;
  assert.equal(session.accounts_available, true);
  saved.username = `smoke_${randomUUID().replaceAll("-", "").slice(0, 24)}`;
  const account = await request("/v1/auth/register", { username: saved.username, password: randomUUID() });
  assert.equal(account.player_id, saved.player);
  const { game } = await request("/v1/games", { scenario_id: "command-link-exercise.v1", title: "Durable recovery smoke" });
  saved.game = game.id;
  const prefix = `/v1/games/${saved.game}`;
  const roles = await request(prefix + "/roles");
  const role = roles.find(role => role.name === "Exercise Commander");
  assert.ok(role);
  const held = await request(`${prefix}/roles/${role.id}/claim`, {});
  saved.role = role.id; saved.generation = held.lease_generation;
  await request(prefix + "/start", {});
  const stateUrl = `${prefix}/state?role_id=${saved.role}&lease_generation=${saved.generation}`;
  const deadline = Date.now() + 15_000;
  do {
    saved.tick = (await request(stateUrl)).tick;
    if (saved.tick > 0) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  } while (Date.now() < deadline);
  assert.ok(saved.tick > 0);
  // This temporary artifact contains a test cookie; never print or commit it.
  await writeFile(values.prepare, JSON.stringify(saved), { mode: 0o600 });
  console.log("Recovery smoke prepared; restart the server, then run --verify.");
} else {
  const session = await request("/v1/auth/session");
  assert.equal(session.player_id, saved.player);
  assert.equal(session.username, saved.username);
  const games = await request("/v1/games");
  assert.equal(games.find(game => game.id === saved.game)?.status, "paused");
  const prefix = `/v1/games/${saved.game}`;
  const roles = await request(prefix + "/roles");
  const role = roles.find(role => role.id === saved.role);
  assert.equal(role.held_by_you, true);
  assert.equal(role.lease_generation, saved.generation);
  const recovered = await request(`${prefix}/state?role_id=${saved.role}&lease_generation=${saved.generation}`);
  assert.ok(recovered.tick >= saved.tick);
  await request(prefix + "/start", {});
  await request(prefix + "/pause", {});
  console.log("Recovery smoke passed: account, session, game, role and tick survived restart; host resumed.");
}
