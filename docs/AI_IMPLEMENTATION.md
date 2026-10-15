# Game AI and OpenRouter agents

The **AI Defense Exercise** is an offline, fictional two-sided exercise. Its Red
commander receives delayed sensor reports, assigns the nearest available
interceptor to each known threat, and issues ordinary movement and engagement
orders. Startup interference prevents sensor reports until the radio recovers at
tick 8. White Cell can inspect truth independently of either side's knowledge.

The deterministic defensive policy is opt-in scenario data, not a replacement
for every campaign's existing patrol behavior. Existing saved games keep their
previous rules. This increment provides a defensive air loop, not complete naval,
undersea, ground, cyber, or orbital AI doctrine.

## Play and assign controllers

1. Create **AI Defense Exercise**. Hosting as **White Cell** is useful for
   inspection; hosting as Blue provides the **Blue Aircraft Operator** role.
2. Start the exercise. Red uses its built-in defense policy by default. Move the
   Blue aircraft to challenge its defenses. Red headquarters cannot react to
   contacts before reports arrive.
3. Open **Team & players → AI controllers**. The host or an active White Cell
   referee may assign **Human**, **Built-in AI**, or **External agent** to any
   operational role. Assignment releases its current player and revokes the old
   controller. An external token is displayed once; copy it before closing.
4. The default external response deadline is 30 real seconds; set 1–300 seconds
   before assigning or rotating a token. Rotation revokes the preceding token.
5. White Cell sees cached agent observations and the latest 100 assignment,
   decision and fallback events. Hosting alone exposes controller metadata,
   without automatically showing observations or decision details. Explicitly
   issuing a role token delegates that role's observation and command access.

Human and agent roles can coexist on either side. Agents can approve or deny
requests only after delivery to their assigned decision role. Agent assignment
prevents vacant-role auto-approval. A built-in controller without authored goals
holds existing orders and leaves approvals pending. The built-in defense policy
produces movement, engagement and return orders; human or external controllers
supply authority decisions.

## Run an OpenRouter agent

Use Node 22+ on the machine running the agent. Copy the following settings into
an ignored `.env` file, replacing the two credential placeholders and game ID:

```dotenv
OPENROUTER_BASE_URL=https://openrouter.ai/api/v1
OPENROUTER_API_KEY=replace-with-your-provider-key
OPENROUTER_MODEL=openrouter/free
OPENROUTER_RESPONSE_FORMAT=auto
OPENROUTER_TIMEOUT_MS=25000
OPENROUTER_MIN_INTERVAL_MS=10000
WAW_SERVER_URL=http://127.0.0.1:8000
WAW_AGENT_TOKEN=replace-with-the-role-connection-token
WAW_GAME_ID=replace-with-the-game-uuid
```

For the default Compose edge use `WAW_SERVER_URL=http://127.0.0.1:8080`.
Run from the repository root:

```sh
node --env-file=.env web/scripts/agent-runner.mjs
```

Start one runner per externally controlled role. Stop with Ctrl+C. Provider keys
stay in the external process and are never browser variables or game snapshot
fields. Scoped game-token hashes, rather than plaintext tokens, are saved.
Remote base URLs require HTTPS; plain HTTP is allowed for loopback development.
Base URLs cannot contain credentials, query parameters, or fragments. Requests do
not follow redirects with credentials.

`openrouter/free` selects an available free model. Alternatively set an explicit
OpenRouter model ID ending in `:free`. The runner rejects paid model IDs and never
substitutes a paid fallback or enables provider plugins. Free-model availability
and quotas vary; the runner does not promise unlimited inference or hardcode a
particular available vendor model. See the official
[free router](https://openrouter.ai/docs/guides/routing/routers/free-router),
[API quickstart](https://openrouter.ai/docs/quickstart), and
[usage limits](https://openrouter.ai/docs/api/reference/limits).

The runner posts to `{OPENROUTER_BASE_URL}/chat/completions`, using bearer
`OPENROUTER_API_KEY`. Its separate game connection uses bearer `WAW_AGENT_TOKEN`.
Inference defaults to one request at a time and a ten-second minimum interval.
Operators running multiple processes with the same provider key must account for
their combined quota. Increase the interval or deadline when appropriate.

`OPENROUTER_RESPONSE_FORMAT=auto` initially requests strict JSON schema; if the
provider explicitly rejects that capability, subsequent requests use prompted
JSON with the same local validation. `json_schema` requires strict output;
`json` starts directly in prompted JSON mode. Models can generate every existing
order type: movement, engagement, radar emission, logistics and IFTU.

Malformed, truncated, unknown-action and oversized outputs are discarded. A
transient request may retry once within the original deadline; rate-limit
responses respect `Retry-After`, quota exhaustion enters cooldown, and
401/403 authentication failures stop the runner. Inference aborts at its timeout
or game deadline. Correct credentials and restart after authentication failures.
Logs contain sanitized error categories and model/latency/token-count metadata,
not provider response bodies or credentials.

## Decision protocol version 1

| Endpoint | Authentication | Behavior |
| --- | --- | --- |
| `GET /v1/games/{game}/agents` | Browser session | Host/referee controller metadata; detailed history and observations require active White Cell |
| `POST /v1/games/{game}/agents/{role}` | Browser session + CSRF | `{ "mode": "human" / "deterministic" / "external", "timeout_ms": 30000 }`; external returns its one-time token |
| `GET /v1/games/{game}/agent/work` | Scoped bearer | `{ "work": observation-or-null }`; reads the cached request without renewing it |
| `POST /v1/games/{game}/agent/decisions` | Scoped bearer | Accepts the versioned decision, returning ordered per-action results |

Observations contain the role's `RoleProjection`, controlled-unit IDs, authored
goals and goal assignments, available authority action keys, role-known authority
requests, and up to 32 recent role-visible intent receipts. They carry
`protocol_version`, `observation_id`, `assignment_generation`, `role_id`, `side`,
and `deadline_unix_ms`. Scoped track IDs never identify hidden enemy entities.
Polling cannot refresh the frozen observation or learn unreported remote state.

A decision echoes protocol, observation and assignment IDs, provides a fresh
`decision_id`, and supplies `actions`, optional `explanation`, and optional
`model`. Actions are tagged:

```json
{
  "type": "order",
  "unit_id": "00000000-0000-0000-0000-000000000033",
  "kind": { "Move": { "north_mps": 0, "east_mps": 35 } }
}
```

Other actions are `select_goal` with `goal_id`/`unit_ids`, and
`authority_decision` with `request_id`/`approve`. Goal selection activates the
built-in executor for those units between external decisions. Explicit accepted
orders remove that unit's goal assignment. Goals must be authored for the role;
agents cannot inject executable doctrine or alter game truth.

The provider returns only `actions` and `explanation`; the runner supplies trusted
protocol/request identifiers and a new decision ID. `agent-protocol.mjs` exports
`proposalSchema` and its validator. `agent-runner.mjs` exports `run`, `infer`,
`gameRequest`, and `configuration`; `run` accepts a `decide` callback for other
providers or deterministic test agents. The Rust protocol is defined in
`sim-ai::agent`, which takes projections and has no simulation-world access.

Limit: 32 actions and 64 KiB per decision, with explanation limited to 2,048 UTF-8
bytes. Actions are evaluated in order and may succeed or fail independently.
The same decision ID and identical payload returns the original result;
conflicting reuse is rejected. Reassignment/authority changes invalidate old
assignment generations. Closed or expired observations reject new decisions.
Engagement and approval IDs must appear in the frozen observation, and commands
are revalidated against current authority and execution rules. An accepted order
means queued execution, not delivery, execution, acknowledgement or impact.

## Scheduling, fallback and recovery

The built-in policy evaluates at most once per game second and replans every
five game seconds or on newer delivered contact observations. It orders goals by
priority, then threats by distance to the defended area, and allocates nearest
eligible aircraft with seeded stable tie-breaking. It ignores stale, unidentified,
friendly and assessed-destroyed tracks. Known low fuel, ammunition exhaustion or
authored health thresholds trigger return to the authored home. A goal may name
`home_facility_id` to use the ordinary logistics recovery command. Missing
logistics status does not establish readiness in logistics-enabled projections.
Aircraft without a threat station at the defended point. These are fictional
kinematic area-defense rules, not a comprehensive tactical or flight model.

The most specific AI command role reserves each unit even while its external
controller is thinking. All proposals use human-equivalent authority, frozen
engagement designations, communications delivery and execution checks. Pending
orders and authority requests suppress duplicate unit tasking.

External decisions do not block simulation time. On timeout the request closes
and deterministic fallback starts; roles without goals preserve current orders.
The controller continues receiving fresh requests. A valid response restores
external control. Pausing, authority edits and reassignment invalidate work.
A response based on an earlier game tick can still be accepted before its wall
deadline, but ordinary track-age and execution rules remain in force.

Existing optional PostgreSQL saves preserve controller configuration, hashed
credentials, goal assignments, accepted decisions, pending orders and duplicate
results. Wall time is an explicit recorded input for external work/fallback
transitions. Recovery never performs inference. Restart pauses the game and
invalidates outstanding work; tokens remain valid until rotated/revoked. Resume
creates fresh observations. No database migration or persistence redesign is
required. Existing scenarios without authored goals retain the legacy planner.

## Validation

Run `npm run test:agents` from `web/` for the mocked provider/runner tests, and
`npm run test:e2e -- agents.e2e.ts` for desktop and phone flows using a local
OpenRouter-compatible provider and the real runner/server. Rust tests cover
knowledge isolation, allocation, delivery, authority, duplicate decisions, timeout
replay, and legacy snapshots. An isolated PostgreSQL regression verifies accepted
decisions and tokens across restart without repeating commands.

These checks require no live OpenRouter calls. A real-provider smoke test remains
optional and requires configured credentials. No long soak is part of this
increment's validation.

Final checkpoint validation (2026-10-14): 310 fast Rust tests, 20 isolated
PostgreSQL tests, 54 frontend tests and 12 runner/provider tests pass, as do strict
Clippy, formatting, dependency-pin verification, frontend/E2E TypeScript checks
and the production frontend build. The full browser run passed 67 cases, skipped
the dedicated-database case and found two default-scenario regressions; after
fixing the lobby default, all 10 agent/gameplay cases passed. The remaining
previously passing cases were not rerun after that UI correction. Desktop and
phone agent cases include a held inference request while the game clock advances.
Live-provider quality/availability and long endurance remain untested.
