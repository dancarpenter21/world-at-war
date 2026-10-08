import assert from "node:assert/strict";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: { "base-url": { type: "string", default: "http://127.0.0.1:8080" } } });
const base = values["base-url"];
const origin = process.env.SMOKE_ORIGIN ?? new URL(base).origin;
function client() {
  let cookie = "", csrf = "";
  return async (path, method = "GET", body, expected = 200) => {
    const response = await fetch(new URL(path, base), {
      method, headers: { "content-type": "application/json", origin, cookie, "x-csrf-token": csrf },
      body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(10_000)
    });
    assert.equal(response.status, expected, `${method} ${path} returned HTTP ${response.status}`);
    const setCookie = response.headers.get("set-cookie");
    if (setCookie) cookie = setCookie.split(";")[0];
    const data = await response.json();
    if (data.csrf_token) csrf = data.csrf_token;
    return data;
  };
}
const host = client(), observer = client();
await host("/v1/auth/guest", "POST", { display_name: "Observer smoke host" });
const guest = await observer("/v1/auth/guest", "POST", { display_name: "Observer smoke guest" });
const { game } = await host("/v1/games", "POST", { scenario_id: "regional-campaign.v1", title: "Observer smoke" });
const prefix = `/v1/games/${game.id}`;
const seat = (await host(prefix + "/roles")).find(role => role.observer_kind === "monitor");
assert.ok(seat, "scenario observer fixture was not embedded in the production image");
assert.equal(seat.claimable, false);
await host(`${prefix}/truth?role_id=${seat.id}&lease_generation=0`, "GET", undefined, 403);
await observer(prefix + "/join", "POST", { display_name: "Observer smoke guest" });
await observer(prefix + "/participants", "GET", undefined, 403);
const grant = `${prefix}/roles/${seat.id}/observer-grant`;
await host(grant, "PUT", { target_player_id: guest.player_id });
const lease = await observer(`${prefix}/roles/${seat.id}/claim`, "POST", {});
assert.equal(lease.observer_kind, "monitor");
assert.equal(lease.command_units, undefined);
await host(prefix + "/pause", "POST", {});
const query = new URLSearchParams({ role_id: seat.id, lease_generation: String(lease.lease_generation) });
const truth = await observer(`${prefix}/truth?${query}`);
assert.ok(truth.units.some(unit => unit.side === "Red"));
assert.ok(truth.communication_links.length > 0);
assert.equal(truth.messages, undefined);
await observer(`${prefix}/state?${query}`, "GET", undefined, 404);
await observer(prefix + "/start", "POST", {}, 403);
await host(grant, "DELETE", {});
await observer(`${prefix}/truth?${query}`, "GET", undefined, 403);
await observer(`${prefix}/roles/${seat.id}/resume`, "POST", {}, 403);
await observer("/v1/auth/logout", "POST", {});
await host("/v1/auth/logout", "POST", {});
console.log("Observer smoke passed: scenario seats, host grants, read-only truth, and paused revocation.");
