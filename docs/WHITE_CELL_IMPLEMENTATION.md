# White Cell teams

White Cell is a neutral team of exercise referees. Each member has their own
identity and leased seat; there is no shared account and no fixed referee count.
Every scenario supports White Cell, including games created before this feature.

## Host or join

Choose **Join as → White Cell** when creating a game. The creator automatically
joins White Cell and opens its workspace, where **Start scenario** starts play
without a combatant role. Blue and Red players can join through the game lobby.

For an existing lobby, running or paused game, select **Join game**, select the
game, and choose **Request White Cell**. The host or a connected White Cell referee
approves the request through **Team & players**. Until approval, the applicant has
no referee access. A player with no prior team can join Blue or available human Red
immediately, then claim an operational role.

**Red control** at creation offers the scenario default and explicit AI/Human
choices in Regional Joint Campaign and Global Crisis. Human mode makes authored
Red command roles playable and disables their AI planner. Disconnecting does not
switch them back to AI. Other scenarios retain their authored roles and clearly
show unavailable Red participation. Changing Red control during play is outside
this increment.

## Referee tools

- **Truth map:** actual units on all sides, weapon positions or pending impacts,
  fuel/ammunition/damage, jamming and network links/queues. Weather inspection and
  editing reuse the existing controller workflow when sensing is configured.
- **Plans & orders:** unpublished, published and received campaign plans, reports,
  clearances, handoffs/proposals, authority decisions and submitted intents.
- **Messages:** current retained message contents, lifecycle and delivery timings.
  This does not introduce raw packet export or a historical archive browser.
- **Player picture:** inspect an operational role's current projection and plans
  without claiming its role. Inspection does not deliver truth to that role.
- **Team notebook:** private, attributed referee notes up to 4,000 characters.
  Notes are append-only; write a correction as another note.

Active referees can start, pause, resume and choose the shared speed from 0.25×,
1×, 2×, 5×, 10×, 30× or 60×. Changing speed while paused leaves the game paused.
Host lifecycle powers remain available independently of their selected team.
Concurrent control changes carry a revision; a stale change asks the caller to
refresh. The Space Assets preview clock remains independent.

## Membership and access

A player selects one team per game. Existing role APIs also bind a player's first
operational claim to its side, preserving the earlier direct claim workflow.
Transfers require the host or another active referee; the host may authorize their
own transfer. Accepted transfers release previous roles and observer grants before
enabling the destination. White Cell-to-combatant transfers warn that previously
seen information cannot be forgotten. Leaving a team does not bypass approval for
returning or changing teams.

The host can remove members; members can leave their own team. This immediately
invalidates server authorization for previous seats, REST requests and streams.
Open clients clear privileged data on invalidation. Disconnecting or locally
leaving the scenario view does not remove team membership; existing heartbeat,
lease reservation and reconnect behavior applies. Referee powers require an
active lease, not merely approved membership.

Older controller/monitor seats remain separately granted, limited observers and
are not promoted automatically. White Cell grants are managed through teams,
not the legacy observer-grant API. No referee seat appears in combatant authority.

## API

All endpoints use the existing authenticated session and CSRF protection. Do not
send `player_id` or `host_player_id` as asserted identity. Privileged queries use
the caller's referee `role_id` and `lease_generation`.

| Endpoint | Purpose |
| --- | --- |
| `POST /v1/games` | Adds optional `creator_team` (`blue`, `red`, `white_cell`) and `red_control` (`human`, `ai`); omission preserves scenario defaults. |
| `GET /v1/games/{id}/teams` | Own team, referee seat, capabilities and request status; managers also see the roster and pending requests. |
| `POST /v1/games/{id}/teams/requests` | Request membership or a transfer using `request_id` and `team`. |
| `POST /v1/games/{id}/teams/requests/{request_id}/decision` | Host/active referee approves or declines with `approve`. |
| `POST /v1/games/{id}/teams/requests/{request_id}/cancel` | Cancel the caller's pending request. |
| `DELETE /v1/games/{id}/teams/members/{player_id}` | Host removes a member, or a member leaves their own team. |
| Existing role claim/resume/renew/release routes | Acquire and maintain individually granted referee seats. |
| `GET /v1/games/{id}/white-cell` | Referee role list, shared notebook and control revision. |
| `GET /v1/games/{id}/white-cell/inspect` | `section=plans` for plans/orders; `target_role_id` for a player picture; otherwise retained messages. |
| `POST /v1/games/{id}/white-cell/notes` | Append a note using `role_id`, `lease_generation`, `note_id` and `text`. |
| Existing truth/environment routes | Authorized truth map/stream and weather controls. |
| Existing start/pause/time-scale routes | Host or active referee controls; updated clients supply `expected_control_revision`. |

Request/note IDs prevent duplicate effects on retries. Game summaries add control
revision, side-specific role availability and explicit Red control; scenario
summaries advertise `human_red_supported`. Operational role lists follow selected
team membership; hosts retain authority administration visibility.

## Save compatibility and validation

Team state is an optional game snapshot field, omitted for untouched legacy games.
Normal existing save/recovery preserves memberships, dynamic referee seats, requests,
notes and Red settings. Running saved games still recover paused. No new database
schema or stronger durability requirement is introduced.

Router regressions cover admission, multiple referees, controls/conflicts, notes,
revocation, role-picture isolation, human Red command execution, transfers and
legacy snapshot compatibility. An isolated PostgreSQL regression covers restart
with saved White Cell state. Real-server Chromium tests cover hosting with both
combatant teams, late/paused joining, shared speed and notes, reload, individual
revocation, approved transfers and narrow-screen layout.

New exercise injects, outcome overrides, announcement delivery and interactive
replay remain later increments. The current workspace exposes available modeled
state and retained messages; it does not reconstruct information already discarded.

Validation for this increment: 251 fast Rust tests, 18 isolated PostgreSQL tests,
47 frontend tests and the production frontend build pass. Strict workspace Clippy,
formatting, dependency-pin verification and browser TypeScript checking pass.
The full Chromium suite passes 63 cases with its dedicated-database browser case
skipped; both White Cell flows pass again after the control-error feedback change.
The fast performance harness passes 22 cases with its opt-in container case
skipped. [Verified screenshots](gameplay-checkpoints.md#white-cell-team) show the
workspace and phone layout. No long soak or development-database migration was run.
