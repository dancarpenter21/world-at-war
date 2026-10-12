//! Private capture and terminal-local metadata projections, separate from canonical state.
use super::*;
use sim_core::audit::{Event, Kind};
use std::collections::VecDeque;

const CAPACITY: usize = 10_000;

#[derive(Default)]
pub(super) struct Runtime {
    pub pending: Vec<Event>,
    pub heads: BTreeMap<Uuid, u64>,
    pub coverage_tick: u64,
    pub partial_history: bool,
    pub history: VecDeque<(Uuid, Metadata)>,
    pub floors: BTreeMap<Uuid, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Metadata {
    pub sequence: u64,
    pub kind: Kind,
    pub at_ns: u64,
    pub message_id: Option<String>,
    pub packet_id: Option<u64>,
    pub fragment: Option<usize>,
    pub attempt: Option<u8>,
    pub hop: Option<usize>,
    pub acknowledgement: bool,
    pub bytes: usize,
    pub reason: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct Query {
    lease_generation: u64,
    player_id: Uuid,
    role_id: Uuid,
    pub cursor: Option<u64>,
    pub limit: Option<usize>,
    pub message_id: Option<String>,
    pub packet_id: Option<u64>,
    pub kind: Option<Kind>,
}
#[derive(Serialize)]
pub(super) struct Page {
    pub events: Vec<Metadata>,
    pub next_cursor: Option<u64>,
    pub coverage_start_tick: u64,
    pub partial_history: bool,
    pub truncated: bool,
}
impl Query {
    pub fn limit(&self) -> usize {
        self.limit.unwrap_or(100).clamp(1, 500)
    }
    pub fn matches(&self, event: &Metadata) -> bool {
        event.sequence > self.cursor.unwrap_or(0)
            && self
                .message_id
                .as_ref()
                .is_none_or(|m| event.message_id.as_ref() == Some(m))
            && self.packet_id.is_none_or(|p| event.packet_id == Some(p))
            && self.kind.is_none_or(|k| event.kind == k)
    }
}

pub(super) fn collect(game: &mut Game) {
    game.audit.pending.extend(game.simulation.drain_audit());
    game.audit.pending.sort_by_key(|event| event.at_ns);
}
pub(super) fn message_key(event: &Event) -> String {
    event
        .context
        .message_id
        .clone()
        .unwrap_or_else(|| format!("packet:{}", event.packet_id.unwrap_or(0)))
}
pub(super) struct Projection {
    pub events: Vec<(Uuid, usize, Metadata)>,
    pub heads: BTreeMap<Uuid, u64>,
}

/// Prepare proposed heads without publishing uncommitted sequences.
pub(super) fn prepare(game: &Game, events: &[Event]) -> Projection {
    let mut heads = game.audit.heads.clone();
    let mut terminals: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    for role in game.roles.values() {
        terminals
            .entry(role.location_unit_id)
            .or_default()
            .push(role.id);
    }
    let mut projected = Vec::new();
    for (ordinal, event) in events.iter().enumerate() {
        for role_id in event
            .terminal
            .and_then(|t| terminals.get(&t))
            .into_iter()
            .flatten()
        {
            let head = heads.entry(*role_id).or_default();
            *head = head.checked_add(1).expect("audit role sequence exhausted");
            projected.push((
                *role_id,
                ordinal,
                Metadata {
                    sequence: *head,
                    kind: event.kind,
                    at_ns: event.at_ns,
                    message_id: event.context.message_id.clone(),
                    packet_id: event.packet_id,
                    fragment: event.context.fragment,
                    attempt: event.context.attempt,
                    // Hop numbers reveal hidden route length; expose local observations only.
                    hop: None,
                    acknowledgement: event.context.acknowledgement,
                    bytes: event.bytes,
                    reason: event.reason.clone(),
                },
            ));
        }
    }
    Projection {
        events: projected,
        heads,
    }
}
pub(super) fn flush_volatile(game: &mut Game) {
    collect(game);
    let events = std::mem::take(&mut game.audit.pending);
    let prepared = prepare(game, &events);
    game.audit.heads = prepared.heads;
    for (role, _, event) in prepared.events {
        game.audit.history.push_back((role, event));
    }
    while game.audit.history.len() > CAPACITY {
        if let Some((role, event)) = game.audit.history.pop_front() {
            game.audit.floors.insert(role, event.sequence);
        }
    }
}
pub(super) fn retention_safe(game: &Game) -> bool {
    if !game.audit.partial_history {
        return true;
    }
    let network = game.simulation.retention_statistics().network;
    // A legacy snapshot may contain late packets whose application associations
    // were retired before this audit existed. Wait for a drained network.
    network.pending_events == 0 && network.pending_receptions == 0
}
pub(super) fn active_messages(game: &Game) -> Vec<String> {
    let mut active = game.simulation.active_audit_messages();
    active.extend(
        game.network_messages
            .iter()
            .filter(|m| matches!(m.state, MessageState::Queued | MessageState::InTransit))
            .map(|m| m.message.id.to_string()),
    );
    active.extend(game.pending_deliveries.keys().map(ToString::to_string));
    active.extend(
        game.simulation
            .audit_correlations()
            .keys()
            .map(|p| format!("packet:{p}")),
    );
    active.into_iter().collect()
}
pub(super) fn transition(game: &mut Game, record: &NetworkMessageRecord) {
    let local = record.message.header.origin_entity_id == record.message.header.recipient_entity_id;
    // Remote physical outcomes are captured at the actual observing device, not at the issuer.
    let kind = match record.state {
        MessageState::Queued => Some(Kind::Accepted),
        MessageState::Delivered if local => Some(Kind::Delivered),
        MessageState::Dropped if record.packet_id.is_none() => Some(Kind::Dropped),
        _ => None,
    };
    if let Some(kind) = kind {
        game.audit.pending.push(Event {
            physical: None,
            kind,
            at_ns: tick_time(record.message.header.created_tick),
            packet_id: record.packet_id,
            bytes: record.encoded_bytes.len(),
            context: sim_core::audit::Context {
                message_id: Some(record.message.id.to_string()),
                ..Default::default()
            },
            terminal: Some(record.message.header.origin_entity_id),
            reason: record.drop_reason.as_ref().map(|_| "send_failed".into()),
        });
    }
}
fn tick_time(tick: u64) -> u64 {
    tick.saturating_mul(sim_core::TICK_SECONDS)
        .saturating_mul(1_000_000_000)
}

pub(super) async fn list(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<Query>,
) -> ApiResult<Page> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    let role = authorized_role(
        game,
        &ProjectionQuery {
            lease_generation: query.lease_generation,
            player_id: query.player_id,
            role_id: query.role_id,
            after_sequence: None,
        },
    )?
    .clone();
    if let Some(store) = &state.durability.store {
        return store
            .audit_page(game_id, role.id, &query)
            .await
            .map(Json)
            .map_err(|_| storage::unavailable());
    }
    flush_volatile(game);
    let mut matching = game
        .audit
        .history
        .iter()
        .filter(|(id, e)| *id == role.id && query.matches(e));
    let events: Vec<_> = matching
        .by_ref()
        .take(query.limit())
        .map(|(_, e)| e.clone())
        .collect();
    let next_cursor = events
        .last()
        .filter(|_| matching.next().is_some())
        .map(|e| e.sequence);
    Ok(Json(Page {
        events,
        next_cursor,
        coverage_start_tick: game.audit.coverage_tick,
        partial_history: game.audit.partial_history,
        truncated: game
            .audit
            .floors
            .get(&role.id)
            .is_some_and(|floor| query.cursor.unwrap_or(0) < *floor),
    }))
}

pub(super) fn retention_days(value: Option<&str>) -> anyhow::Result<u32> {
    let value = value.unwrap_or("0");
    anyhow::ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "COMMUNICATIONS_ARCHIVE_RETENTION_DAYS must be a nonnegative integer"
    );
    Ok(value.parse()?)
}
pub(super) async fn maintenance(state: AppState, days: u32) {
    maintenance_loop(state, days, Duration::from_secs(60)).await;
}
#[cfg(test)]
pub(super) async fn maintenance_for_test(state: AppState, days: u32) {
    maintenance_loop(state, days, Duration::from_millis(5)).await;
}
async fn maintenance_loop(state: AppState, days: u32, cadence: Duration) {
    if days == 0 || state.durability.store.is_none() {
        return;
    }
    let mut interval = tokio::time::interval(cadence);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        // Drain eligible history in bounded transactions. Mutex fairness and an
        // explicit yield let gameplay mutations run between batches.
        loop {
            let gate = state.durability.gate.lock().await;
            if state
                .durability
                .failed
                .load(std::sync::atomic::Ordering::Acquire)
                || state
                    .durability
                    .stopping
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                break;
            }
            let games = state.games.read().await;
            let span = performance::Span::new("audit_cleanup", || serde_json::json!({}));
            let store = state.durability.store.as_ref().unwrap();
            let result = store.cleanup_audit(&games, days).await;
            let backlog = if result.as_ref().is_ok_and(|rows| *rows < 1000) {
                store.audit_backlog(&games, days).await.ok()
            } else {
                None
            };
            drop(games);
            drop(gate);
            let more = matches!(result, Ok(1000));
            match result {
                Ok(rows) => performance::record(
                    || serde_json::json!({"kind":"audit_cleanup_result", "deleted_rows":rows, "failed":false, "eligible_backlog_rows":backlog.map(|b| b.0), "oldest_eligible_age_seconds":backlog.map(|b| b.1)}),
                ),
                Err(_) => {
                    eprintln!("Communications archive cleanup failed; retrying next interval");
                    performance::record(
                        || serde_json::json!({"kind":"audit_cleanup_result", "failed":true}),
                    );
                }
            }
            span.finish();
            if !more {
                break;
            }
            tokio::task::yield_now().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authority_tests::game_for;

    #[test]
    fn retention_configuration_is_opt_in_and_strict() {
        assert_eq!(retention_days(None).unwrap(), 0);
        assert_eq!(retention_days(Some("0")).unwrap(), 0);
        assert_eq!(retention_days(Some("30")).unwrap(), 30);
        for value in ["", "-1", " 30", "1.5", "4294967296"] {
            assert!(retention_days(Some(value)).is_err());
        }
    }
    #[test]
    fn audit_has_terminal_local_sequences_and_never_reveals_routes() {
        let game = game_for(command_link_exercise_scenario());
        let role = game.roles.values().next().unwrap().clone();
        let events = vec![
            Event {
                physical: None,
                kind: Kind::Delivered,
                at_ns: 1,
                packet_id: Some(42),
                bytes: 30,
                context: sim_core::audit::Context {
                    from: Some(role.location_unit_id),
                    to: Some(Uuid::from_u128(999)),
                    hop: Some(7),
                    ..Default::default()
                },
                terminal: Some(Uuid::from_u128(999)),
                reason: None,
            },
            Event {
                physical: None,
                kind: Kind::Scheduled,
                at_ns: 2,
                packet_id: Some(43),
                bytes: 31,
                context: sim_core::audit::Context {
                    hop: Some(8),
                    ..Default::default()
                },
                terminal: Some(role.location_unit_id),
                reason: None,
            },
        ];
        let projected = prepare(&game, &events).events;
        let local: Vec<_> = projected
            .iter()
            .filter(|(id, _, _)| *id == role.id)
            .collect();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].2.sequence, 1);
        assert_eq!(local[0].2.kind, Kind::Scheduled);
        assert_eq!(local[0].2.hop, None);
        let json = serde_json::to_value(&local[0].2).unwrap();
        assert!(
            json.get("from").is_none() && json.get("to").is_none() && json.get("payload").is_none()
        );
    }
    #[test]
    fn prepared_projection_preserves_shared_terminal_order_without_publishing_heads() {
        let mut game = game_for(command_link_exercise_scenario());
        let mut roles = game.roles.values().cloned().collect::<Vec<_>>();
        roles.sort_by_key(|role| role.id);
        let terminal = roles[0].location_unit_id;
        game.roles.get_mut(&roles[1].id).unwrap().location_unit_id = terminal;
        game.audit.heads.insert(roles[0].id, 7);
        let before = game.audit.heads.clone();
        let event = |packet, terminal| Event {
            physical: None,
            kind: Kind::Received,
            at_ns: 1,
            packet_id: Some(packet),
            bytes: 20,
            context: Default::default(),
            terminal,
            reason: Some("local".into()),
        };
        let events = vec![
            event(10, Some(terminal)),
            event(11, None),
            event(12, Some(terminal)),
        ];
        let prepared = prepare(&game, &events);
        assert_eq!(game.audit.heads, before);
        assert_eq!(prepared.events.len(), 4);
        assert_eq!(
            prepared
                .events
                .iter()
                .map(|(role, ordinal, e)| (*role, *ordinal, e.sequence, e.packet_id))
                .collect::<Vec<_>>(),
            vec![
                (roles[0].id, 0, 8, Some(10)),
                (roles[1].id, 0, 1, Some(10)),
                (roles[0].id, 2, 9, Some(12)),
                (roles[1].id, 2, 2, Some(12))
            ]
        );
        assert_eq!(prepared.heads[&roles[0].id], 9);
        let empty = prepare(&game, &[]);
        assert!(empty.events.is_empty());
        assert_eq!(empty.heads, before);
        // The volatile publisher uses exactly the same preparation and stable sort.
        game.audit.pending = events;
        flush_volatile(&mut game);
        assert_eq!(game.audit.heads, prepared.heads);
        assert_eq!(
            game.audit
                .history
                .iter()
                .map(|(_, e)| e.packet_id)
                .collect::<Vec<_>>(),
            vec![Some(10), Some(10), Some(12), Some(12)]
        );
    }
    #[test]
    fn audit_buffers_do_not_change_snapshot_hashes_and_eviction_is_explicit() {
        let mut game = game_for(command_link_exercise_scenario());
        let before = recovery::encode(&game).unwrap();
        let role = game.roles.values().next().unwrap().clone();
        for n in 0..CAPACITY + 1 {
            game.audit.pending.push(Event {
                physical: None,
                kind: Kind::Attempt,
                at_ns: n as u64,
                packet_id: None,
                bytes: 0,
                context: Default::default(),
                terminal: Some(role.location_unit_id),
                reason: None,
            });
        }
        flush_volatile(&mut game);
        assert_eq!(game.audit.history.len(), CAPACITY);
        assert!(game.audit.floors.contains_key(&role.id));
        assert_eq!(before, recovery::encode(&game).unwrap());
    }
    #[test]
    fn fragmented_reports_are_captured_once_and_correlate_after_restore() {
        let mut game = game_for(regional_campaign_scenario());
        game.simulation.enable_audit();
        // Use an authored role-to-role route, avoiding external catalogs.
        let endpoints = game
            .roles
            .values()
            .map(|r| r.location_unit_id)
            .collect::<Vec<_>>();
        let pair = endpoints
            .iter()
            .find_map(|from| {
                endpoints
                    .iter()
                    .find(|to| *to != from && game.simulation.has_message_route(*from, **to))
                    .map(|to| (*from, *to))
            })
            .unwrap();
        let id = Uuid::from_u128(88001);
        game.simulation
            .send_message(id, pair.0, pair.1, vec![7; 1300], 50)
            .unwrap();
        game.simulation
            .send_message(id, pair.0, pair.1, vec![7; 1300], 50)
            .unwrap();
        game.simulation.step();
        let contexts = game.simulation.audit_correlations().clone();
        let snapshot = game.simulation.snapshot();
        let mut restored = sim_core::Simulation::restore(snapshot).unwrap();
        restored.restore_audit_correlations(contexts.clone());
        assert_eq!(
            serde_json::to_value(restored.audit_correlations()).unwrap(),
            serde_json::to_value(contexts).unwrap()
        );
        let mut events = game.simulation.drain_audit();
        for _ in 0..20 {
            restored.step();
            events.extend(restored.drain_audit());
        }
        let ours: Vec<_> = events
            .iter()
            .filter(|e| e.context.message_id.as_deref() == Some(&id.to_string()))
            .collect();
        assert_eq!(ours.iter().filter(|e| e.kind == Kind::Accepted).count(), 1);
        assert_eq!(
            ours.iter()
                .filter(|e| e.kind == Kind::DuplicateSuppressed)
                .count(),
            1
        );
        assert!(ours.iter().any(|e| e.context.fragment == Some(2)));
        let mut seen = BTreeSet::new();
        for e in ours.iter().filter(|e| e.kind == Kind::TransmissionStarted) {
            assert!(seen.insert((e.packet_id, e.at_ns, e.terminal)));
        }
    }
}
