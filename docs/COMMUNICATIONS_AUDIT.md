# Communications metadata audit and archive retention

The server records C2, internal knowledge-report, reliable-fragment,
acknowledgement and IFTU communications metadata. Physical events are captured
before their transport handlers consume them; application events distinguish
acceptance, attempts, reassembly, acknowledgement, duplicate suppression,
rejection and termination. Raw packet payloads are not archived by this feature.
Existing C2 message bodies continue to be stored once in the lifecycle archive.

## Durability and recovery

With PostgreSQL, private observations, terminal-local projections and packet
correlations commit in the same transaction as their game boundary. Records are
identified by game, committed boundary and observation ordinal. Failed writes
roll back the game and audit together and use the existing fail-closed pause.
Recovery verifies the existing snapshots/journal without rewriting audit history,
then restores the independent correlation state before resuming traffic.

Audit buffers, counters and correlation metadata are outside canonical snapshots
and hashes. Existing snapshot schemas and the dependency pin remain compatible.
Migration 006 is additive and does not rewrite previous migrations. Games upgraded
without previous audit coverage report `partial_history: true` and the tick at
which new capture began. Pending associations recover from available C2,
fragment and IFTU state; earlier events are not reconstructed.

Without PostgreSQL, a shared per-game buffer holds up to 10,000 terminal-local
observations. Eviction advances each affected role's truncation marker. This
history is lost on restart. The existing volatile C2 JSONL log remains separate
and is not subject to the PostgreSQL retention setting.

## Authorized metadata API

`GET /v1/games/{game_id}/network/audit?role_id=...&lease_generation=...`
requires the authenticated session cookie and a current operational-role lease.
The server supplies player identity from that session. Host ownership and an
observer grant confer no access to this API.

Optional filters are `message_id`, `packet_id` and `kind`. `cursor` selects
observations after a role-local sequence; `limit` defaults to 100 and is clamped
to 1–500. Message identifiers use C2/reliable UUID strings, `iftu:<number>` for
IFTU traffic, and `intent:<uuid>` for duplicate command submissions.

The response contains:

- `events`: sequence, kind, virtual `at_ns`, message/packet identifiers, fragment
  and attempt where available, acknowledgement flag, byte count and safe reason.
- `next_cursor`: the last returned sequence when another matching page exists;
  otherwise null. For polling, retain the last returned event sequence.
- `coverage_start_tick` and `partial_history`: the capture coverage boundary.
- `truncated`: requested history includes observations removed by eviction or
  retention. It does not disclose other roles' removed history.

Kinds are `accepted`, `duplicate_suppressed`, `attempt`, `scheduled`,
`transmission_started`, `received`, `delivered`, `dropped`, `expired`,
`reassembled`, `acknowledged`, `unacknowledged`, `reporting_ended` and `rejected`.

An event is visible only at its observing terminal. Intermediate nodes see their
own hop observations. A sender sees its own sends and any returned acknowledgement,
not remote reception or reassembly. A receiver's acknowledgement retries remain
receiver-local. A terminal may observe a packet before application reassembly,
but the API exposes no message content or track information.

Physical device/channel identifiers, complete routes, remote endpoints, payloads
and forwarding-hop positions remain private. The metadata `hop` field is null.
Private physical records retain device/channel metadata and modeled transmission
rate/arrival time when provided by c3mesh. Existing C2 event endpoints retain
content-specific visibility rules and add a role-specific `truncated` field.

## Retention operation

`COMMUNICATIONS_ARCHIVE_RETENTION_DAYS=0` (the default, also used when unset)
keeps PostgreSQL communications history indefinitely. A positive integer enables
age-based cleanup. Invalid or overflowing integers fail startup. Compose forwards
this setting; direct runs may set it in their environment.

Cleanup runs every 60 seconds, including when games are paused, and drains eligible
history through successive batches while yielding to gameplay between them. It deletes up to
1,000 archive event rows per transaction, plus dependent terminal projections and
up to 1,000 unreferenced C2 bodies per processed game. Age uses database commit
time, not simulation time. Migration gives legacy C2 rows an age starting at
upgrade, so enabling retention does not immediately erase existing history.

Only inactive histories whose latest activity in both communications archives is
older than the cutoff qualify. Pending deliveries, retries, acknowledgements,
fragments and physical receptions protect their associations. For partially
covered legacy games, cleanup conservatively waits for a drained physical network;
continuous traffic can therefore defer cleanup. Bodies survive while either
archive still references their message. Partial batches update role-specific
truncation markers in the same transaction as deletion.

Maintenance uses the runtime durability gate and releases it between batches.
Cleanup failure rolls back the batch, logs a maintenance error and retries next
interval. It does not pause otherwise healthy gameplay. Required capture writes
continue to fail closed. Recovery journals, checkpoints, current message state,
authority records, receipts and duplicate-suppression IDs are never pruned here.
This policy does not promise bounded total memory or database size.

## Validation and measurements

Rust regressions cover terminal filtering, bounded buffers, fragmented reports,
IFTU capture, canonical hash preservation, acknowledgement visibility, database
migration, restart correlation, transactional write failures, pending-traffic
protection, cross-archive age protection, partial cleanup and maintenance rollback.
Browser gameplay checks inspect the audit API and filtered pagination alongside
the existing network workflow.

Performance traces contain aggregate `audit_events`/`audit_bytes` per committed
boundary and pending/history/correlation counts in retention samples. Maintenance
adds cleanup duration, deleted rows, failures, eligible backlog and oldest eligible
age. These fields contain no payloads. The analyzer preserves audit totals and
cleanup samples; the unattended runner includes them in its JSON report.

The short harness accepts `--archive-retention-days N` and records it in metadata;
its default is 0. See [load-testing operation](performance/LOAD_TESTING.md) for
short profiles and separately requested long-soak acceptance.
