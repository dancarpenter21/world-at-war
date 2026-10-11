//! Durable records for the single authoritative runtime. No connection URL or
//! credential is included in operational errors.
use super::*;
use anyhow::Context;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};

const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_identity.sql"),
    include_str!("../migrations/002_games.sql"),
    include_str!("../migrations/003_network_audit.sql"),
    include_str!("../migrations/004_game_metadata.sql"),
    include_str!("../migrations/005_event_version.sql"),
    include_str!("../migrations/006_communications_audit.sql"),
];
const RUNTIME_LOCK: i64 = 0x57415752554e;

const COMMIT_SQL: &[(&str, &str)] = &[
    ("audit_records", "INSERT INTO communications_audit(game_id,boundary,ordinal,message_key,detail) SELECT $1,$2,* FROM UNNEST($3::BIGINT[],$4::TEXT[],$5::JSONB[])"),
    ("audit_roles", "INSERT INTO communications_audit_roles(game_id,role_id,sequence,boundary,ordinal,detail) SELECT $1,role_id,sequence,$2,ordinal,detail FROM UNNEST($3::UUID[],$4::BIGINT[],$5::BIGINT[],$6::JSONB[]) AS r(role_id,sequence,ordinal,detail)"),
    ("audit_runtime", "INSERT INTO communications_audit_runtime(game_id,correlations,heads,coverage_tick,partial_history) VALUES($1,$2,$3,$4,$5) ON CONFLICT(game_id) DO UPDATE SET correlations=EXCLUDED.correlations,heads=EXCLUDED.heads WHERE (communications_audit_runtime.correlations,communications_audit_runtime.heads) IS DISTINCT FROM (EXCLUDED.correlations,EXCLUDED.heads)"),

    ("delete_session", "DELETE FROM browser_sessions WHERE token_hash = $1"),
    ("insert_session", "INSERT INTO browser_sessions(token_hash, player_id, csrf_token, expires_unix) VALUES ($1, $2, $3, $4)"),
    ("insert_game", "INSERT INTO games(id, sequence, snapshot_sequence, snapshot, snapshot_sha256, current_sha256) VALUES ($1, $2, $2, $3, $4, $4)"),
    ("checkpoint_game", "UPDATE games SET sequence = $2, current_sha256 = $3, snapshot_sequence = $2, snapshot = $4, snapshot_sha256 = $3 WHERE id = $1 AND sequence = $2::BIGINT - 1"),
    ("advance_game", "UPDATE games SET sequence = $2, current_sha256 = $3 WHERE id = $1 AND sequence = $2::BIGINT - 1"),
    ("journal", "INSERT INTO game_events(game_id, sequence, kind, actor, operation, payload, state_sha256) VALUES ($1, $2, $3, $4, $5, $6, $7)"),
    ("metadata", "INSERT INTO game_metadata(game_id,title,host_id,scenario_id,scenario_version,scenario_checksum,status,tick) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(game_id) DO UPDATE SET title=EXCLUDED.title,status=EXCLUDED.status,tick=EXCLUDED.tick"),
    ("messages", "INSERT INTO network_messages(game_id,message_id,profile_id,origin_role,origin_entity,recipient_entity,content,encoded) SELECT $1, * FROM UNNEST($2::uuid[],$3::text[],$4::uuid[],$5::uuid[],$6::uuid[],$7::jsonb[],$8::bytea[]) ON CONFLICT(game_id,message_id) DO NOTHING"),
    ("events", "INSERT INTO network_events(game_id,sequence,message_id,detail) SELECT $1, * FROM UNNEST($2::bigint[],$3::uuid[],$4::jsonb[])"),
    ("memberships", "INSERT INTO game_memberships(game_id,player_id,display_name) SELECT $1, * FROM UNNEST($2::uuid[],$3::text[]) ON CONFLICT(game_id,player_id) DO UPDATE SET display_name=EXCLUDED.display_name WHERE game_memberships.display_name IS DISTINCT FROM EXCLUDED.display_name"),
    ("seats", "INSERT INTO game_seats(game_id,seat_id,owner_id,grant_id,generation,lease) SELECT $1, * FROM UNNEST($2::uuid[],$3::uuid[],$4::uuid[],$5::bigint[],$6::jsonb[]) ON CONFLICT(game_id,seat_id) DO UPDATE SET owner_id=EXCLUDED.owner_id,grant_id=EXCLUDED.grant_id,generation=EXCLUDED.generation,lease=EXCLUDED.lease WHERE (game_seats.owner_id,game_seats.grant_id,game_seats.generation,game_seats.lease) IS DISTINCT FROM (EXCLUDED.owner_id,EXCLUDED.grant_id,EXCLUDED.generation,EXCLUDED.lease)"),
];

pub(super) struct Store {
    client: Mutex<Client>,
    statements: BTreeMap<&'static str, tokio_postgres::Statement>,
}

pub(super) struct StoredSession {
    pub token_hash: String,
    pub player_id: Uuid,
    pub display_name: String,
    pub username: Option<String>,
    pub csrf_token: String,
    pub expires_unix: u64,
}

pub(super) struct Account {
    pub player_id: Uuid,
    pub display_name: String,
    pub username: String,
    pub password_hash: String,
}

pub(super) struct StoredEvent {
    pub sequence: u64,
    pub kind: String,
    pub payload: Vec<u8>,
    pub hash: String,
}
pub(super) struct StoredRun {
    pub id: Uuid,
    pub sequence: u64,
    pub snapshot_sequence: u64,
    pub snapshot: Vec<u8>,
    pub snapshot_hash: String,
    pub hash: String,
    pub events: Vec<StoredEvent>,
}

impl Store {
    pub async fn audit_page(
        &self,
        game: Uuid,
        role: Uuid,
        query: &audit::Query,
    ) -> anyhow::Result<audit::Page> {
        let client = self.client.lock().await;
        let kind = query.kind.map(|k| {
            serde_json::to_value(k)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        });
        let packet = query.packet_id.map(|p| p.to_string());
        let rows = client.query("SELECT detail FROM communications_audit_roles WHERE game_id=$1 AND role_id=$2 AND sequence>$3 AND ($4::TEXT IS NULL OR detail->>'message_id'=$4) AND ($5::TEXT IS NULL OR detail->>'packet_id'=$5) AND ($6::TEXT IS NULL OR detail->>'kind'=$6) ORDER BY sequence LIMIT $7", &[&game, &role, &i64::try_from(query.cursor.unwrap_or(0)).unwrap_or(i64::MAX), &query.message_id, &packet, &kind, &i64::try_from(query.limit()+1)?]).await?;
        let mut events: Vec<audit::Metadata> = rows
            .into_iter()
            .map(|r| serde_json::from_value(r.get(0)))
            .collect::<Result<_, _>>()?;
        let more = events.len() > query.limit();
        events.truncate(query.limit());
        let next_cursor = events.last().filter(|_| more).map(|e| e.sequence);
        let meta = client
            .query_opt(
                "SELECT coverage_tick, floors, partial_history FROM communications_audit_runtime WHERE game_id=$1",
                &[&game],
            )
            .await?;
        let (coverage_start_tick, floor, partial_history) = if let Some(meta) = meta {
            let floors: BTreeMap<Uuid, u64> = serde_json::from_value(meta.get(1))?;
            (
                u64::try_from(meta.get::<_, i64>(0))?,
                floors.get(&role).copied().unwrap_or(0),
                meta.get::<_, bool>(2),
            )
        } else {
            (0, 0, false)
        };
        Ok(audit::Page {
            events,
            next_cursor,
            coverage_start_tick,
            partial_history,
            truncated: query.cursor.unwrap_or(0) < floor,
        })
    }
    pub async fn load_audit(&self, game: &mut Game) -> anyhow::Result<()> {
        let row = self.client.lock().await.query_opt("SELECT correlations,heads,coverage_tick,partial_history FROM communications_audit_runtime WHERE game_id=$1", &[&game.id]).await?;
        let correlations = if let Some(row) = row {
            game.audit.heads = serde_json::from_value(row.get(1))?;
            game.audit.coverage_tick = u64::try_from(row.get::<_, i64>(2))?;
            game.audit.partial_history = row.get(3);
            serde_json::from_value(row.get(0))?
        } else {
            game.audit.coverage_tick = game.simulation.tick();
            game.audit.partial_history = true;
            BTreeMap::new()
        };
        game.audit.pending.clear();
        game.simulation.restore_audit_correlations(correlations);
        // Pre-upgrade C2 snapshots preserve their own packet associations.
        for (packet, message) in &game.packet_messages {
            if !game.simulation.audit_correlations().contains_key(packet) {
                if let Some(record) = game
                    .network_messages
                    .iter()
                    .find(|r| r.message.id == *message)
                {
                    game.simulation.annotate_audit_packet(
                        *packet,
                        sim_core::audit::Context {
                            message_id: Some(message.to_string()),
                            from: Some(record.message.header.origin_entity_id),
                            to: Some(record.message.header.recipient_entity_id),
                            ..Default::default()
                        },
                    );
                }
            }
        }
        Ok(())
    }
    pub async fn lifecycle_floor(&self, game: Uuid, role: Uuid) -> anyhow::Result<u64> {
        let row = self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT lifecycle_floors FROM communications_audit_runtime WHERE game_id=$1",
                &[&game],
            )
            .await?;
        let floors: BTreeMap<Uuid, u64> = row
            .map(|r| serde_json::from_value(r.get(0)))
            .transpose()?
            .unwrap_or_default();
        Ok(floors.get(&role).copied().unwrap_or(0))
    }
    /// One bounded maintenance batch; callers release the durability gate between batches.
    pub async fn cleanup_audit(
        &self,
        games: &BTreeMap<Uuid, Game>,
        days: u32,
    ) -> anyhow::Result<u64> {
        if days == 0 {
            return Ok(0);
        }
        let mut client = self.client.lock().await;
        let tx = client.transaction().await?;
        let mut deleted = 0u64;
        for game in games.values() {
            if !audit::retention_safe(game) {
                continue;
            }
            if deleted >= 1000 {
                break;
            }
            let active = audit::active_messages(game);
            let remaining = i64::try_from(1000 - deleted)?;
            let rows = tx.query("SELECT boundary,ordinal FROM communications_audit WHERE game_id=$1 AND message_key IN (SELECT message_key FROM communications_audit WHERE game_id=$1 AND NOT(message_key=ANY($2)) GROUP BY message_key HAVING MAX(recorded_at)<CURRENT_TIMESTAMP-($3::BIGINT * INTERVAL '1 day') AND NOT EXISTS(SELECT 1 FROM network_events n WHERE n.game_id=$1 AND n.message_id::TEXT=communications_audit.message_key AND n.recorded_at>=CURRENT_TIMESTAMP-($3::BIGINT * INTERVAL '1 day'))) ORDER BY boundary,ordinal LIMIT $4", &[&game.id,&active,&i64::from(days),&remaining]).await?;
            let boundaries: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
            let ordinals: Vec<i64> = rows.iter().map(|r| r.get(1)).collect();
            if !rows.is_empty() {
                let meta = tx.query_one("SELECT floors FROM communications_audit_runtime WHERE game_id=$1 FOR UPDATE", &[&game.id]).await?;
                let mut floors: BTreeMap<Uuid, u64> = serde_json::from_value(meta.get(0))?;
                let removed = tx.query("SELECT role_id,MAX(sequence) FROM communications_audit_roles r JOIN UNNEST($2::BIGINT[],$3::BIGINT[]) AS d(boundary,ordinal) USING(boundary,ordinal) WHERE r.game_id=$1 GROUP BY role_id", &[&game.id,&boundaries,&ordinals]).await?;
                for row in removed {
                    let role: Uuid = row.get(0);
                    let floor = u64::try_from(row.get::<_, i64>(1))?;
                    floors
                        .entry(role)
                        .and_modify(|v| *v = (*v).max(floor))
                        .or_insert(floor);
                }
                tx.execute(
                    "UPDATE communications_audit_runtime SET floors=$2 WHERE game_id=$1",
                    &[&game.id, &serde_json::to_value(floors)?],
                )
                .await?;
                deleted += tx.execute("DELETE FROM communications_audit a USING UNNEST($2::BIGINT[],$3::BIGINT[]) AS d(boundary,ordinal) WHERE a.game_id=$1 AND a.boundary=d.boundary AND a.ordinal=d.ordinal", &[&game.id,&boundaries,&ordinals]).await?;
            }
            if deleted >= 1000 {
                break;
            }
            let remaining = i64::try_from(1000 - deleted)?;
            let rows = tx.query("SELECT sequence,message_id,detail FROM network_events WHERE game_id=$1 AND message_id IN (SELECT message_id FROM network_events WHERE game_id=$1 AND NOT(message_id::TEXT=ANY($2)) GROUP BY message_id HAVING MAX(recorded_at)<CURRENT_TIMESTAMP-($3::BIGINT * INTERVAL '1 day') AND NOT EXISTS(SELECT 1 FROM communications_audit a WHERE a.game_id=$1 AND a.message_key=network_events.message_id::TEXT AND a.recorded_at>=CURRENT_TIMESTAMP-($3::BIGINT * INTERVAL '1 day'))) ORDER BY sequence LIMIT $4", &[&game.id,&active,&i64::from(days),&remaining]).await?;
            let sequences: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
            if !sequences.is_empty() {
                let row = tx.query_one("SELECT lifecycle_floors FROM communications_audit_runtime WHERE game_id=$1 FOR UPDATE", &[&game.id]).await?;
                let mut floors: BTreeMap<Uuid, u64> = serde_json::from_value(row.get(0))?;
                for row in &rows {
                    let message: Uuid = row.get(1);
                    let detail: serde_json::Value = row.get(2);
                    let mut record = game
                        .network_messages
                        .iter()
                        .find(|r| r.message.id == message)
                        .ok_or_else(|| {
                            anyhow::anyhow!("missing live message for archive retention")
                        })?
                        .clone();
                    record.sequence = u64::try_from(row.get::<_, i64>(0))?;
                    record.state = serde_json::from_value(detail["state"].clone())?;
                    record.delivered_at_ns =
                        serde_json::from_value(detail["delivered_at_ns"].clone())?;
                    for role in game
                        .roles
                        .values()
                        .filter(|role| network_message_visible(&record, role))
                    {
                        floors
                            .entry(role.id)
                            .and_modify(|v| *v = (*v).max(record.sequence))
                            .or_insert(record.sequence);
                    }
                }
                tx.execute(
                    "UPDATE communications_audit_runtime SET lifecycle_floors=$2 WHERE game_id=$1",
                    &[&game.id, &serde_json::to_value(floors)?],
                )
                .await?;
                deleted += tx
                    .execute(
                        "DELETE FROM network_events WHERE game_id=$1 AND sequence=ANY($2)",
                        &[&game.id, &sequences],
                    )
                    .await?;
            }
            tx.execute("DELETE FROM network_messages WHERE game_id=$1 AND message_id IN (SELECT m.message_id FROM network_messages m WHERE m.game_id=$1 AND NOT(m.message_id::TEXT=ANY($2)) AND NOT EXISTS(SELECT 1 FROM network_events e WHERE e.game_id=m.game_id AND e.message_id=m.message_id) AND NOT EXISTS(SELECT 1 FROM communications_audit a WHERE a.game_id=m.game_id AND a.message_key=m.message_id::TEXT) LIMIT 1000)", &[&game.id,&active]).await?;
        }
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn audit_backlog(
        &self,
        games: &BTreeMap<Uuid, Game>,
        days: u32,
    ) -> anyhow::Result<(u64, u64)> {
        if days == 0 {
            return Ok((0, 0));
        }
        let client = self.client.lock().await;
        let mut count = 0;
        let mut oldest = 0;
        for game in games.values() {
            if !audit::retention_safe(game) {
                continue;
            }
            let active = audit::active_messages(game);
            for sql in [
                "SELECT COALESCE(SUM(n),0)::BIGINT,COALESCE(MAX(EXTRACT(EPOCH FROM CURRENT_TIMESTAMP-first_at)),0)::BIGINT FROM (SELECT COUNT(*) AS n,MIN(recorded_at) AS first_at FROM communications_audit WHERE game_id=$1 AND NOT(message_key=ANY($2)) GROUP BY message_key HAVING MAX(recorded_at)<CURRENT_TIMESTAMP-($3::BIGINT*INTERVAL '1 day') AND NOT EXISTS(SELECT 1 FROM network_events n WHERE n.game_id=$1 AND n.message_id::TEXT=communications_audit.message_key AND n.recorded_at>=CURRENT_TIMESTAMP-($3::BIGINT*INTERVAL '1 day'))) e",
                "SELECT COALESCE(SUM(n),0)::BIGINT,COALESCE(MAX(EXTRACT(EPOCH FROM CURRENT_TIMESTAMP-first_at)),0)::BIGINT FROM (SELECT COUNT(*) AS n,MIN(recorded_at) AS first_at FROM network_events WHERE game_id=$1 AND NOT(message_id::TEXT=ANY($2)) GROUP BY message_id HAVING MAX(recorded_at)<CURRENT_TIMESTAMP-($3::BIGINT*INTERVAL '1 day') AND NOT EXISTS(SELECT 1 FROM communications_audit a WHERE a.game_id=$1 AND a.message_key=network_events.message_id::TEXT AND a.recorded_at>=CURRENT_TIMESTAMP-($3::BIGINT*INTERVAL '1 day'))) e"
            ] {
                let row = client.query_one(sql, &[&game.id,&active,&i64::from(days)]).await?;
                count += u64::try_from(row.get::<_,i64>(0))?;
                oldest = oldest.max(u64::try_from(row.get::<_,i64>(1))?);
            }
        }
        Ok((count, oldest))
    }

    pub async fn runs(&self) -> anyhow::Result<Vec<StoredRun>> {
        let client = self.client.lock().await;
        let rows = client.query("SELECT id, sequence, snapshot_sequence, snapshot, snapshot_sha256, current_sha256 FROM games ORDER BY id", &[]).await?;
        let mut runs = Vec::new();
        for row in rows {
            let id: Uuid = row.get(0);
            let snapshot_sequence: i64 = row.get(2);
            let events = client.query("SELECT sequence, kind, payload, state_sha256, schema_version FROM game_events WHERE game_id = $1 AND sequence > $2 ORDER BY sequence", &[&id, &snapshot_sequence]).await?
                .into_iter().map(|row| { anyhow::ensure!(row.get::<_, i32>(4) == 1, "unsupported game event schema"); Ok(StoredEvent { sequence: u64::try_from(row.get::<_, i64>(0))?, kind: row.get(1), payload: row.get(2), hash: row.get(3) }) }).collect::<anyhow::Result<_>>()?;
            runs.push(StoredRun {
                id,
                sequence: u64::try_from(row.get::<_, i64>(1))?,
                snapshot_sequence: u64::try_from(snapshot_sequence)?,
                snapshot: row.get(3),
                snapshot_hash: row.get(4),
                hash: row.get(5),
                events,
            });
        }
        Ok(runs)
    }

    pub async fn commit(
        &self,
        changes: &[recovery::Boundary],
        games: &mut BTreeMap<Uuid, Game>,
        identity: Option<&auth::SessionUpdate>,
    ) -> anyhow::Result<()> {
        for change in changes {
            audit::collect(games.get_mut(&change.id).unwrap());
        }
        let commit = performance::Span::new("persistence_commit", || {
            json!({
                "boundaries": changes.len(),
            "game_ids": changes.iter().map(|c| c.id).collect::<Vec<_>>(),
                "checkpoints": changes.iter().filter(|c| c.checkpoint).count(),
                "journal_bytes": changes.iter().map(|c| c.payload.len()).sum::<usize>(),
                // Only checkpoints submit snapshot bytes to PostgreSQL.
                "snapshot_submitted_bytes": changes.iter().filter(|c| c.checkpoint || c.sequence == 1).map(|c| c.state.len()).sum::<usize>(),
                "audit_events": changes.iter().map(|c| games[&c.id].audit.pending.len()).sum::<usize>(),
                "audit_bytes": changes.iter().map(|c| serde_json::to_vec(&games[&c.id].audit.pending).map_or(0, |b| b.len())).sum::<usize>(),
                "archived_events": changes.iter().map(|c| games[&c.id].network_message_events.len()).sum::<usize>()
            })
        });
        let wait = performance::Span::new("persistence_lock", || json!({}));
        let mut client = self.client.lock().await;
        wait.finish();
        let sql = performance::Span::new("persistence_sql", || json!({}));
        let transaction = client.transaction().await?;
        if let Some(identity) = identity {
            if let Some(previous) = &identity.previous {
                transaction
                    .execute(&self.statements["delete_session"], &[previous])
                    .await?;
            }
            if let Some(session) = &identity.session {
                let session = auth::stored(session);
                transaction
                    .execute(
                        &self.statements["insert_session"],
                        &[
                            &session.token_hash,
                            &session.player_id,
                            &session.csrf_token,
                            &i64::try_from(session.expires_unix)?,
                        ],
                    )
                    .await?;
            }
        }
        for change in changes {
            let count = if change.sequence == 1 {
                transaction
                    .execute(
                        &self.statements["insert_game"],
                        &[&change.id, &change.sequence, &change.state, &change.hash],
                    )
                    .await?
            } else if change.checkpoint {
                transaction
                    .execute(
                        &self.statements["checkpoint_game"],
                        &[&change.id, &change.sequence, &change.hash, &change.state],
                    )
                    .await?
            } else {
                transaction
                    .execute(
                        &self.statements["advance_game"],
                        &[&change.id, &change.sequence, &change.hash],
                    )
                    .await?
            };
            anyhow::ensure!(count == 1, "stale game journal sequence");
            transaction
                .execute(
                    &self.statements["journal"],
                    &[
                        &change.id,
                        &change.sequence,
                        &change.kind,
                        &change.actor,
                        &change.operation,
                        &change.payload,
                        &change.hash,
                    ],
                )
                .await?;
            let game = games.get_mut(&change.id).unwrap();
            let captured = game.audit.pending.clone();
            let projected = audit::project(game, &captured);
            if !captured.is_empty() {
                let ordinals: Vec<i64> = (0..captured.len())
                    .map(|n| i64::try_from(n).unwrap())
                    .collect();
                let keys: Vec<_> = captured.iter().map(audit::message_key).collect();
                let details = captured
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()?;
                transaction
                    .execute(
                        &self.statements["audit_records"],
                        &[&game.id, &change.sequence, &ordinals, &keys, &details],
                    )
                    .await?;
            }
            if !projected.is_empty() {
                let roles: Vec<_> = projected.iter().map(|(r, _, _)| *r).collect();
                let sequences: Vec<i64> = projected
                    .iter()
                    .map(|(_, _, e)| i64::try_from(e.sequence).unwrap())
                    .collect();
                let ordinals: Vec<i64> = projected
                    .iter()
                    .map(|(_, n, _)| i64::try_from(*n).unwrap())
                    .collect();
                let details = projected
                    .iter()
                    .map(|(_, _, e)| serde_json::to_value(e))
                    .collect::<Result<Vec<_>, _>>()?;
                transaction
                    .execute(
                        &self.statements["audit_roles"],
                        &[
                            &game.id,
                            &change.sequence,
                            &roles,
                            &sequences,
                            &ordinals,
                            &details,
                        ],
                    )
                    .await?;
            }
            transaction
                .execute(
                    &self.statements["audit_runtime"],
                    &[
                        &game.id,
                        &serde_json::to_value(game.simulation.audit_correlations())?,
                        &serde_json::to_value(&game.audit.heads)?,
                        &i64::try_from(game.audit.coverage_tick)?,
                        &game.audit.partial_history,
                    ],
                )
                .await?;
            let game = &*game;
            let status = match game.status {
                GameStatus::Lobby => "lobby",
                GameStatus::Running => "running",
                GameStatus::Paused => "paused",
            };
            transaction
                .execute(
                    &self.statements["metadata"],
                    &[
                        &game.id,
                        &game.title,
                        &game.host,
                        &game.scenario_id,
                        &i64::from(game.scenario_version),
                        &game.scenario_checksum,
                        &status,
                        &i64::try_from(game.simulation.tick())?,
                    ],
                )
                .await?;
            // Archive every ordered transition, but serialize each message body once.
            let mut messages = BTreeMap::new();
            let mut event_sequences = Vec::new();
            let mut event_messages = Vec::new();
            let mut event_details = Vec::new();
            for record in &game.network_message_events {
                messages.entry(record.message.id).or_insert(record);
                // Match the persisted lifecycle fields without constructing and
                // then discarding another copy of the body and byte vector.
                let detail = json!({
                    "sequence": record.sequence,
                    "state": record.state,
                    "packet_id": record.packet_id,
                    "started_at_ns": record.started_at_ns,
                    "terminal_at_ns": record.terminal_at_ns,
                    "delivered_at_ns": record.delivered_at_ns,
                    "drop_reason": record.drop_reason,
                });
                event_sequences.push(i64::try_from(record.sequence)?);
                event_messages.push(record.message.id);
                event_details.push(detail);
            }
            if !messages.is_empty() {
                let records: Vec<_> = messages.values().collect();
                let ids: Vec<_> = records.iter().map(|r| r.message.id).collect();
                let profiles: Vec<_> = records
                    .iter()
                    .map(|r| r.message.profile_id.as_str())
                    .collect();
                let roles: Vec<_> = records
                    .iter()
                    .map(|r| r.message.header.origin_role_id)
                    .collect();
                let origins: Vec<_> = records
                    .iter()
                    .map(|r| r.message.header.origin_entity_id)
                    .collect();
                let recipients: Vec<_> = records
                    .iter()
                    .map(|r| r.message.header.recipient_entity_id)
                    .collect();
                let contents = records
                    .iter()
                    .map(|r| serde_json::to_value(&r.message))
                    .collect::<Result<Vec<_>, _>>()?;
                let encoded: Vec<_> = records.iter().map(|r| r.encoded_bytes.as_slice()).collect();
                transaction
                    .execute(
                        &self.statements["messages"],
                        &[
                            &game.id,
                            &ids,
                            &profiles,
                            &roles,
                            &origins,
                            &recipients,
                            &contents,
                            &encoded,
                        ],
                    )
                    .await?;
                transaction
                    .execute(
                        &self.statements["events"],
                        &[&game.id, &event_sequences, &event_messages, &event_details],
                    )
                    .await?;
            }
            // Bulk upserts retain the same transaction and sequence fence. Avoid
            // new row versions (and WAL) when membership or seat data is unchanged.
            let players: Vec<_> = game.members.iter().copied().collect();
            let names: Vec<_> = players
                .iter()
                .map(|id| {
                    game.participant_names
                        .get(id)
                        .map(String::as_str)
                        .unwrap_or("Guest")
                })
                .collect();
            if !players.is_empty() {
                transaction
                    .execute(
                        &self.statements["memberships"],
                        &[&game.id, &players, &names],
                    )
                    .await?;
            }
            let seats: Vec<_> = game
                .roles
                .iter()
                .map(|(id, role)| (id, &role.occupancy, None))
                .chain(
                    game.observers
                        .iter()
                        .map(|(id, seat)| (id, &seat.occupancy, seat.granted_player)),
                )
                .collect();
            if !seats.is_empty() {
                let ids: Vec<_> = seats.iter().map(|(id, _, _)| **id).collect();
                let owners: Vec<_> = seats
                    .iter()
                    .map(|(_, occupancy, _)| occupancy.owner)
                    .collect();
                let grants: Vec<_> = seats.iter().map(|(_, _, grant)| *grant).collect();
                let generations = seats
                    .iter()
                    .map(|(_, occupancy, _)| i64::try_from(occupancy.lease_generation))
                    .collect::<Result<Vec<_>, _>>()?;
                let leases = seats
                    .iter()
                    .map(|(_, occupancy, _)| {
                        occupancy
                            .lease
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                transaction
                    .execute(
                        &self.statements["seats"],
                        &[&game.id, &ids, &owners, &grants, &generations, &leases],
                    )
                    .await?;
            }
        }
        sql.finish();
        let flush = performance::Span::new("persistence_flush", || json!({}));
        transaction.commit().await?;
        flush.finish();
        for change in changes {
            games.get_mut(&change.id).unwrap().audit.pending.clear();
            // Historical lifecycle records now live in the archive. Current
            // message states and duplicate-suppression IDs remain in snapshots.
            games
                .get_mut(&change.id)
                .unwrap()
                .network_message_events
                .clear();
        }
        commit.finish();
        Ok(())
    }

    pub async fn network_events(
        &self,
        game: Uuid,
        cursor: u64,
        role: &Role,
        query: &NetworkEventsQuery,
    ) -> anyhow::Result<Vec<NetworkMessageRecord>> {
        let state = query.state.map(|state| {
            serde_json::to_value(state)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        });
        let rows = self.client.lock().await.query(
            "SELECT m.content, m.encoded, e.detail FROM network_events e JOIN network_messages m USING(game_id,message_id) WHERE e.game_id=$1 AND e.sequence>$2 AND (m.origin_role=$3 OR m.recipient_entity=$4 OR m.origin_entity=$4) AND ($5::TEXT IS NULL OR m.profile_id=$5) AND ($6::TEXT IS NULL OR e.detail->>'state'=$6) ORDER BY e.sequence LIMIT 500",
            &[&game, &i64::try_from(cursor).unwrap_or(i64::MAX), &role.id, &role.location_unit_id, &query.profile_id, &state],
        ).await?;
        rows.into_iter()
            .map(|row| {
                let mut detail: serde_json::Value = row.get(2);
                let object = detail
                    .as_object_mut()
                    .ok_or_else(|| anyhow::anyhow!("invalid network event"))?;
                object.insert("message".into(), row.get(0));
                object.insert(
                    "encoded_bytes".into(),
                    serde_json::to_value(row.get::<_, Vec<u8>>(1))?,
                );
                Ok(serde_json::from_value(detail)?)
            })
            .collect()
    }

    pub async fn from_environment() -> anyhow::Result<Option<Arc<Self>>> {
        let Some(url) = std::env::var("DATABASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
        else {
            return Ok(None);
        };
        Self::connect(&url).await.map(|s| Some(Arc::new(s)))
    }

    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let (mut client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .map_err(|_| anyhow::anyhow!("PostgreSQL connection failed"))?;
        tokio::spawn(async move {
            if connection.await.is_err() {
                eprintln!("PostgreSQL connection lost; durable mutations are unavailable");
            }
        });
        // Keep this session-level lock for the lifetime of the connection. A
        // second process must never advance another copy of the same runtime.
        let held: bool = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&RUNTIME_LOCK])
            .await?
            .get(0);
        anyhow::ensure!(held, "another server owns this database's game runtime");
        let transaction = client.transaction().await?;
        transaction.batch_execute("CREATE TABLE IF NOT EXISTS schema_migrations (version INTEGER PRIMARY KEY, checksum TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP)").await?;
        let versions = transaction
            .query(
                "SELECT version, checksum FROM schema_migrations ORDER BY version",
                &[],
            )
            .await?;
        anyhow::ensure!(
            versions.len() <= MIGRATIONS.len(),
            "database schema is newer than this server"
        );
        for (index, migration) in MIGRATIONS.iter().enumerate() {
            let version = (index + 1) as i32;
            let checksum = format!("{:x}", Sha256::digest(migration.as_bytes()));
            if let Some(row) = versions.get(index) {
                anyhow::ensure!(
                    row.get::<_, i32>(0) == version && row.get::<_, String>(1) == checksum,
                    "database schema migration is incompatible"
                );
            } else {
                transaction
                    .batch_execute(migration)
                    .await
                    .context("apply database schema")?;
                transaction
                    .execute(
                        "INSERT INTO schema_migrations(version, checksum) VALUES ($1, $2)",
                        &[&version, &checksum],
                    )
                    .await?;
            }
        }
        transaction.commit().await?;
        let mut statements = BTreeMap::new();
        for (name, sql) in COMMIT_SQL {
            statements.insert(*name, client.prepare(sql).await?);
        }
        Ok(Self {
            client: Mutex::new(client),
            statements,
        })
    }

    pub async fn sessions(&self, now: u64) -> anyhow::Result<Vec<StoredSession>> {
        let client = self.client.lock().await;
        client
            .execute(
                "DELETE FROM browser_sessions WHERE expires_unix <= $1",
                &[&i64::try_from(now)?],
            )
            .await?;
        let rows = client.query(
            "SELECT s.token_hash, s.player_id, p.display_name, p.username, s.csrf_token, s.expires_unix FROM browser_sessions s JOIN players p ON p.id = s.player_id WHERE s.expires_unix > $1",
            &[&i64::try_from(now)?],
        ).await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredSession {
                    token_hash: row.get(0),
                    player_id: row.get(1),
                    display_name: row.get(2),
                    username: row.get(3),
                    csrf_token: row.get(4),
                    expires_unix: u64::try_from(row.get::<_, i64>(5))?,
                })
            })
            .collect()
    }

    pub async fn insert_session(&self, session: &StoredSession, guest: bool) -> anyhow::Result<()> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await?;
        if guest {
            transaction
                .execute(
                    "INSERT INTO players(id, display_name) VALUES ($1, $2)",
                    &[&session.player_id, &session.display_name],
                )
                .await?;
        }
        transaction.execute(
            "INSERT INTO browser_sessions(token_hash, player_id, csrf_token, expires_unix) VALUES ($1, $2, $3, $4)",
            &[&session.token_hash, &session.player_id, &session.csrf_token, &i64::try_from(session.expires_unix)?],
        ).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn account(&self, username: &str) -> anyhow::Result<Option<Account>> {
        Ok(self
            .client
            .lock()
            .await
            .query_opt(
                "SELECT id, display_name, username, password_hash FROM players WHERE username = $1",
                &[&username],
            )
            .await?
            .map(|row| Account {
                player_id: row.get(0),
                display_name: row.get(1),
                username: row.get(2),
                password_hash: row.get(3),
            }))
    }

    /// Register the authenticated guest without changing player identity. The
    /// unique username and guest-only update are enforced by the transaction.
    pub async fn register(
        &self,
        player: Uuid,
        username: &str,
        password_hash: &str,
    ) -> anyhow::Result<bool> {
        let changed = self.client.lock().await.execute(
            "UPDATE players SET username = $2, password_hash = $3 WHERE id = $1 AND username IS NULL",
            &[&player, &username, &password_hash],
        ).await;
        match changed {
            Ok(count) => Ok(count == 1),
            Err(error)
                if error.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) =>
            {
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }
}

pub(super) fn unavailable() -> (StatusCode, Json<ErrorResponse>) {
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
        "Durable storage is unavailable; retry when the server recovers",
    )
}
