# Durable accounts and game recovery

PostgreSQL persistence is enabled when `DATABASE_URL` is set. Compose supplies a
PostgreSQL 17 service and a named data volume. Direct runs without a database keep
the existing volatile guest/game behavior. A configured database must be available
and compatible before the server becomes ready.

## Identity and transactions

Guests may save their current player identity as an account. Usernames are
case-insensitive ASCII letters, digits and underscores (3–40 characters);
passwords contain 12–128 characters. Argon2id hashing and verification run in a
bounded blocking pool. Session cookies contain random secrets; PostgreSQL stores
their SHA-256 hashes, expiry, and CSRF secrets. Passwords and cookie secrets never
enter the game journal.

Login rotates the cookie and CSRF secret. Session replacement/logout and released
roles commit in the same transaction. In-memory sessions change only after that
commit. Requests recheck session validity after acquiring the mutation lock, so a
request queued behind logout cannot reclaim a seat. A disconnected caller does
not cancel a partially executed durable mutation. Clients retry commands using
their original intent ID.

Migrations are transactional and checksummed. A session-level PostgreSQL advisory
lock fences the single authoritative server for a database. An event-write
failure restores the last committed in-memory game state, pauses games and blocks
further mutations. Restore database access and restart the server to recover;
the application never silently switches to volatile storage.

Lease snapshots contain absolute wall-clock deadlines. Restored runtime timers
use monotonic time without extending those deadlines. Expired sessions lose their
seats during recovery; account identity, membership and observer grants remain.

## Snapshots, journal and archive

Queryable game metadata, memberships and seats live in separate tables from the
private deterministic game snapshot. The snapshot includes ECS state, simulation
and radio clocks, complete c3mesh queues/receptions/runtime endpoints, transport
fragments and duplicate suppression, role knowledge, AI attempts, authority
workflows, command receipts, IFTU state and post-mission reporting.

Snapshot schema, event schema, dependency revision, scenario version/content hash
and communications/message-catalog hashes must match. Snapshot and replay-state
SHA-256 checks reject corruption or divergence. JSON uses exact floating-point
round trips. Live orbital data does not replace frozen per-game catalog and
satellite-request context during recovery.

Every accepted game mutation writes a checkpoint containing its inputs/outcomes,
actor and operation. Simulation ticks journal the external occupancy/status input
and deterministic state hash; a checkpoint is saved every 60 journal boundaries.
Pause and clean shutdown also checkpoint. Recovery starts from the latest
checkpoint and replays its suffix through the same AI/simulation schedule. A
corrupt or incompatible run makes startup fail explicitly, without resetting an
existing game ID to tick zero. Running games recover paused; the host resumes
using the existing controls. Lobby games remain in the lobby.

The archive stores each C2 message body once and references it from ordered
lifecycle records. Committed lifecycle history is removed from the live event
buffer. Existing event pagination, profile/state filters and role visibility
rules apply when reading PostgreSQL history. Current message states, receipts,
authority records and duplicate-suppression IDs remain available in game snapshots.
The archive retains history indefinitely; it does not yet implement a retention
policy or a complete audit of every internal knowledge-report/fragment packet.
It also does not claim bounded total application memory.

## Running and recovering

1. Prepare the pinned dependency with `sh scripts/setup-c3mesh.sh` (or the
   PowerShell counterpart) and configure the ignored local `.env` from
   `.env.example`.
2. Start `docker compose up --build`. The database volume survives ordinary
   `docker compose down`; `down --volumes` deletes it.
3. In the lobby choose **Save guest as account** before discarding a guest cookie.
   **Sign in** restores that player ID and game/host access. A lost unsaved guest
   cookie cannot be recovered. There is no password-reset or host-transfer flow.
4. After a restart, select the recovered game and reclaim/resume the appropriate
   seat. Expired reservations are available to other authorized players.

For a direct run, point `DATABASE_URL` at PostgreSQL on the trusted local network.
The current driver uses an unencrypted connection; remote TLS deployment is not
implemented. Use the same source/dependency pins when recovering a backup.
Back up the entire database, including migrations, snapshots, journals, sessions
and archives; copying only metadata cannot recover games. For example:

```sh
docker compose exec -T database pg_dump -U world_at_war -d world_at_war -Fc > /path/outside/repo/world-at-war.dump
```

## Validation

Fast snapshot tests compare complete state and role projections across the
bundled scenarios, queued commands, weapon-provider handoffs, fragmented traffic,
execution acknowledgements and post-mission reporting. Lease tests cover active,
reserved, expired and invalid absolute deadlines.

Database integration tests create and drop a unique database per test. Use a
**dedicated test PostgreSQL instance** and a login with `CREATEDB` permission:

```sh
TEST_DATABASE_URL=postgres://test_user:test_password@127.0.0.1:5432/postgres \
  cargo test --locked -p world-at-war-server -- --ignored
```

These tests cover migrations/fencing, saved identity/session restart, concurrent
claims, queued-request revocation, disconnected callers, observer grants,
expired sessions, transaction failure rollback, corrupted snapshots/journals,
checkpoint replay, duplicate intent retries and authorized archived reads.
Ordinary `cargo test` reports these database tests as ignored; CI runs them in a
separate PostgreSQL job.

The browser crash-recovery test uses a separate, disposable database:

```sh
PERSISTENCE_E2E_DATABASE_URL=postgres://test_user:test_password@127.0.0.1:5432/waw_browser \
  npm run test:e2e --prefix web -- persistence.e2e.ts
```

The test kills its own server, checks restored game/role/tick state, and covers
registration, invalid login, valid login and shared-tab logout. Other browser
fixtures explicitly disable persistence. CI supplies both test databases and
runs production Compose checks. No long soak is part of this increment; previous
performance measurements describe the earlier volatile runtime, and persistence
throughput/storage growth need separate measurement.
