//! Durable boundaries serialize mutations and ticks before publishing their
//! results. Checkpoints preserve accepted external decisions; tick events replay
//! through the same AI and simulation schedule used by the live server.
use super::*;
use axum::{extract::Request, middleware::Next, response::Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

const SCHEMA: u32 = 1;
const DEPENDENCY: &str = include_str!("../../../c3mesh-revision.txt");

#[derive(Default)]
pub(super) struct Durability {
    pub store: Option<Arc<storage::Store>>,
    pub gate: tokio::sync::Mutex<()>,
    pub failed: AtomicBool,
    pub stopping: AtomicBool,
}
impl Durability {
    pub fn new(store: Option<Arc<storage::Store>>) -> Self {
        Self {
            store,
            ..Self::default()
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope<T> {
    schema_version: u32,
    dependency: String,
    game: T,
}

pub(super) fn encode(game: &Game) -> anyhow::Result<Vec<u8>> {
    let mut span = performance::Span::new("persistence_encode", || json!({"game_id": game.id}));
    let bytes = serde_json::to_vec(&Envelope {
        schema_version: SCHEMA,
        dependency: DEPENDENCY.trim().into(),
        game,
    })?;
    span.set("bytes", bytes.len());
    span.finish();
    Ok(bytes)
}
pub(super) fn decode(bytes: &[u8]) -> anyhow::Result<Game> {
    let saved: Envelope<Game> = serde_json::from_slice(bytes)?;
    anyhow::ensure!(
        saved.schema_version == SCHEMA,
        "incompatible game snapshot schema"
    );
    anyhow::ensure!(
        saved.dependency == DEPENDENCY.trim(),
        "incompatible snapshot dependency pin"
    );
    Ok(saved.game)
}
pub(super) fn digest(bytes: &[u8]) -> String {
    let span = performance::Span::new("persistence_hash", || json!({"bytes": bytes.len()}));
    let hash = format!("{:x}", Sha256::digest(bytes));
    span.finish();
    hash
}

#[derive(Serialize, Deserialize)]
pub(super) struct TickInput {
    roles: BTreeMap<Uuid, leases::Occupancy>,
    observers: BTreeMap<Uuid, leases::Occupancy>,
    status: GameStatus,
}
impl TickInput {
    pub fn capture(game: &Game) -> Self {
        Self {
            roles: game
                .roles
                .iter()
                .map(|(id, role)| (*id, role.occupancy.clone()))
                .collect(),
            observers: game
                .observers
                .iter()
                .map(|(id, seat)| (*id, seat.occupancy.clone()))
                .collect(),
            status: game.status,
        }
    }
    pub(super) fn apply(self, game: &mut Game) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.roles.keys().eq(game.roles.keys())
                && self.observers.keys().eq(game.observers.keys()),
            "tick seat schema mismatch"
        );
        for (id, occupancy) in self.roles {
            game.roles.get_mut(&id).unwrap().occupancy = occupancy;
        }
        for (id, occupancy) in self.observers {
            game.observers.get_mut(&id).unwrap().occupancy = occupancy;
        }
        game.status = self.status;
        if game.status == GameStatus::Running {
            ai_orders::process_ai_orders(game);
            advance_game_tick(game);
            intents::record_execution_results(game);
        }
        Ok(())
    }
}

pub(super) struct Boundary {
    pub id: Uuid,
    pub sequence: i64,
    pub kind: &'static str,
    pub actor: Option<Uuid>,
    pub operation: String,
    pub payload: Vec<u8>,
    pub state: Vec<u8>,
    pub hash: String,
    pub checkpoint: bool,
}
impl Boundary {
    pub fn new(
        game: &mut Game,
        input: Option<TickInput>,
        actor: Option<Uuid>,
        operation: &str,
    ) -> anyhow::Result<Self> {
        game.persistence_sequence = game
            .persistence_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("game journal exhausted"))?;
        let state = encode(game)?;
        let hash = digest(&state);
        let checkpoint = input.is_none() || game.persistence_sequence.is_multiple_of(60);
        let (kind, payload) = if let Some(input) = input {
            ("tick", serde_json::to_vec(&input)?)
        } else {
            ("checkpoint", state.clone())
        };
        Ok(Self {
            id: game.id,
            sequence: i64::try_from(game.persistence_sequence)?,
            kind,
            actor,
            operation: operation.into(),
            payload,
            state,
            hash,
            checkpoint,
        })
    }
}

pub(super) fn backup(games: &BTreeMap<Uuid, Game>) -> anyhow::Result<BTreeMap<Uuid, Vec<u8>>> {
    games
        .iter()
        .map(|(id, game)| Ok((*id, encode(game)?)))
        .collect()
}
pub(super) fn rollback(
    state: &AppState,
    games: &mut BTreeMap<Uuid, Game>,
    before: BTreeMap<Uuid, Vec<u8>>,
) {
    games.clear();
    for (id, bytes) in before {
        let mut game = decode(&bytes).expect("locally generated snapshot must restore");
        game.status = GameStatus::Paused;
        game.operational_error = Some("Durable commit failed; game paused. Restart the server after restoring database access.".into());
        games.insert(id, game);
    }
    state.durability.failed.store(true, Ordering::Release);
}

pub(super) async fn boundary(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.durability.store.is_none() {
        return next.run(request).await;
    }
    let mutation = !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    if !mutation {
        return transaction_boundary(State(state), request, next).await;
    }
    // A disconnected caller must not cancel a handler between changing memory
    // and committing its journal. Finish the boundary even if the waiter drops.
    let task_state = state.clone();
    match tokio::spawn(transaction_boundary(State(task_state), request, next)).await {
        Ok(response) => response,
        Err(_) => {
            state.durability.failed.store(true, Ordering::Release);
            for game in state.games.write().await.values_mut() {
                game.status = GameStatus::Paused;
                game.operational_error =
                    Some("Durable request failed; restart the server to recover".into());
            }
            storage::unavailable().into_response()
        }
    }
}

async fn transaction_boundary(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if state.durability.store.is_none() {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let game_request = path == "/v1/games" || path.starts_with("/v1/games/");
    let access_change = path.starts_with("/v1/auth/");
    if !game_request && !access_change {
        return next.run(request).await;
    }
    let wait = performance::Span::new("durability_gate", || json!({"source": "http"}));
    let _gate = state.durability.gate.lock().await;
    wait.finish();
    // Authentication ran before the gate. A concurrent logout may have revoked
    // its copied session while this request was waiting.
    if request
        .extensions()
        .get::<auth::Session>()
        .is_some_and(|session| !state.sessions.valid(session))
    {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "session_required",
            "Session expired or revoked",
        )
        .into_response();
    }
    let mutation = !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    if mutation
        && (state.durability.failed.load(Ordering::Acquire)
            || state.durability.stopping.load(Ordering::Acquire))
    {
        return storage::unavailable().into_response();
    }
    if !mutation {
        return next.run(request).await;
    }
    let boundary = performance::Span::new("mutation_boundary", || {
        json!({
            "operation": if access_change { "auth" } else { "game" },
            "method": request.method().as_str()
        })
    });
    let actor = request
        .extensions()
        .get::<auth::Session>()
        .map(|s| s.player_id);
    let operation = format!("{} {path}", request.method());
    let games_read = state.games.read().await;
    let before = match backup(&games_read) {
        Ok(before) => before,
        Err(_) => return storage::unavailable().into_response(),
    };
    drop(games_read);
    let response = next.run(request).await;
    let identity = response.extensions().get::<auth::SessionUpdate>().cloned();
    let mut games = state.games.write().await;
    let mut changes = Vec::new();
    for game in games.values_mut() {
        let changed = encode(game).map(|bytes| {
            before.get(&game.id) != Some(&bytes)
                || !game.audit.pending.is_empty()
                || game.simulation.has_pending_audit()
        });
        match changed {
            Ok(false) => continue,
            Ok(true) => match Boundary::new(game, None, actor, &operation) {
                Ok(change) => changes.push(change),
                Err(_) => {
                    rollback(&state, &mut games, before);
                    return storage::unavailable().into_response();
                }
            },
            Err(_) => {
                rollback(&state, &mut games, before);
                return storage::unavailable().into_response();
            }
        }
    }
    if (!changes.is_empty() || identity.is_some())
        && state
            .durability
            .store
            .as_ref()
            .unwrap()
            .commit(&changes, &mut games, identity.as_ref())
            .await
            .inspect_err(|error| eprintln!("durable commit failed: {error}"))
            .is_err()
    {
        rollback(&state, &mut games, before);
        return storage::unavailable().into_response();
    }
    if let Some(identity) = identity {
        identity.apply(&state.sessions);
    }
    boundary.finish();
    response
}

pub(super) async fn load(state: &AppState) -> anyhow::Result<()> {
    let Some(store) = &state.durability.store else {
        return Ok(());
    };
    let runs = store.runs().await?;
    let active_players = state.sessions.active_players();
    let mut games = BTreeMap::new();
    for run in runs {
        anyhow::ensure!(
            digest(&run.snapshot) == run.snapshot_hash,
            "snapshot checksum mismatch for {}",
            run.id
        );
        let mut game = decode(&run.snapshot)?;
        anyhow::ensure!(
            game.id == run.id && game.persistence_sequence == run.snapshot_sequence,
            "snapshot identity mismatch"
        );
        for event in run.events {
            anyhow::ensure!(
                event.sequence == game.persistence_sequence + 1,
                "game journal sequence gap"
            );
            match event.kind.as_str() {
                "checkpoint" => {
                    game = decode(&event.payload)?;
                }
                "tick" => {
                    serde_json::from_slice::<TickInput>(&event.payload)?.apply(&mut game)?;
                    game.persistence_sequence = event.sequence;
                }
                _ => anyhow::bail!("unsupported game event type"),
            }
            anyhow::ensure!(
                game.id == run.id
                    && game.persistence_sequence == event.sequence
                    && digest(&encode(&game)?) == event.hash,
                "replay state hash mismatch for {} at event {}",
                run.id,
                event.sequence
            );
        }
        anyhow::ensure!(
            game.persistence_sequence == run.sequence && digest(&encode(&game)?) == run.hash,
            "recovered state does not match journal head"
        );
        let scenario = state
            .scenarios
            .get(&game.scenario_id)
            .ok_or_else(|| anyhow::anyhow!("snapshot scenario unavailable"))?;
        anyhow::ensure!(
            scenario.version == game.scenario_version
                && digest(&serde_json::to_vec(scenario)?) == game.scenario_checksum
                && state.communications_catalog.checksum() == game.communications_catalog_checksum
                && state.communications_catalog.message_pack_checksum()
                    == game.message_pack_checksum,
            "snapshot scenario or catalog incompatible"
        );
        game.network_message_events.clear();
        store.load_audit(&mut game).await?;
        if game.status == GameStatus::Running {
            game.status = GameStatus::Paused;
        }
        let expired_players: Vec<_> = game.members.difference(&active_players).copied().collect();
        for player in expired_players {
            leases::release_player(&mut game, player);
        }
        leases::expire(&mut game, std::time::Instant::now());
        game.network_projection_sequence = game.network_projection_sequence.saturating_add(1);
        // Persist the explicit recovery pause and any reservations that expired
        // during downtime before requests or ticks can observe this runtime.
        let boundary = Boundary::new(&mut game, None, None, "recovery_pause")?;
        let id = game.id;
        games.insert(id, game);
        store.commit(&[boundary], &mut games, None).await?;
    }
    *state.games.write().await = games;
    Ok(())
}

/// Flush a final compatible checkpoint after in-flight boundaries have finished.
pub(super) async fn shutdown(state: &AppState) -> anyhow::Result<()> {
    state.durability.stopping.store(true, Ordering::Release);
    let _gate = state.durability.gate.lock().await;
    let Some(store) = &state.durability.store else {
        return Ok(());
    };
    // A failed runtime must recover from its last committed state on restart.
    if state.durability.failed.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut games = state.games.write().await;
    let mut changes = Vec::new();
    for game in games.values_mut() {
        if game.status == GameStatus::Running {
            game.status = GameStatus::Paused;
        }
        changes.push(Boundary::new(game, None, None, "shutdown_checkpoint")?);
    }
    store.commit(&changes, &mut games, None).await
}

impl Game {
    pub(super) fn next_id(&mut self, purpose: &str) -> Uuid {
        let sequence = self.next_identifier;
        self.next_identifier = self
            .next_identifier
            .checked_add(1)
            .expect("game identifier sequence exhausted");
        Uuid::new_v5(&self.id, format!("{purpose}:{sequence}").as_bytes())
    }
}

pub(super) mod simulation {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &Simulation,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.snapshot().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Simulation, D::Error> {
        Simulation::restore(sim_core::snapshot::SimulationSnapshot::deserialize(
            deserializer,
        )?)
        .map_err(serde::de::Error::custom)
    }
}
pub(super) mod authority_requests {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        values: &BTreeMap<Uuid, AuthorityRequest>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        values
            .iter()
            .map(|(id, request)| (id, request, &request.intent))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<Uuid, AuthorityRequest>, D::Error> {
        let values =
            Vec::<(Uuid, AuthorityRequest, Option<PlayerIntent>)>::deserialize(deserializer)?;
        Ok(values
            .into_iter()
            .map(|(id, mut request, intent)| {
                request.intent = intent;
                (id, request)
            })
            .collect())
    }
}
pub(super) mod submission_result {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        value: &intents::SubmissionResult,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .as_ref()
            .map_err(|(status, error)| (status.as_u16(), &error.0))
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<intents::SubmissionResult, D::Error> {
        let value = Result::<SubmissionOutcome, (u16, ErrorResponse)>::deserialize(deserializer)?;
        Ok(match value {
            Ok(value) => Ok(value),
            Err((status, error)) => Err((
                StatusCode::from_u16(status).map_err(serde::de::Error::custom)?,
                Json(error),
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_and_tick_replay_preserve_canonical_state_and_role_views() {
        for scenario in [
            command_link_exercise_scenario(),
            regional_campaign_scenario(),
            global_crisis_scenario(),
        ] {
            let mut live = authority_tests::game_from_scenario(scenario);
            for tick in 0..8 {
                let before = encode(&live).unwrap();
                let mut restored = decode(&before).unwrap();
                assert_eq!(
                    digest(&before),
                    digest(&encode(&restored).unwrap()),
                    "round trip at {tick}"
                );
                let input = TickInput::capture(&live);
                let replay_input: TickInput =
                    serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
                input.apply(&mut live).unwrap();
                replay_input.apply(&mut restored).unwrap();
                assert_eq!(
                    digest(&encode(&live).unwrap()),
                    digest(&encode(&restored).unwrap()),
                    "tick replay at {tick}"
                );
                for role in live.roles.values() {
                    assert_eq!(
                        serde_json::to_value(
                            live.simulation
                                .projection_for(role.location_unit_id, role.side)
                        )
                        .unwrap(),
                        serde_json::to_value(
                            restored
                                .simulation
                                .projection_for(role.location_unit_id, role.side)
                        )
                        .unwrap(),
                        "role {} at {tick}",
                        role.name
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_incompatible_snapshot_and_dependency_versions() {
        let game = authority_tests::game_from_scenario(command_link_exercise_scenario());
        let original: serde_json::Value = serde_json::from_slice(&encode(&game).unwrap()).unwrap();
        for (field, value) in [
            ("schema_version", serde_json::json!(99)),
            ("dependency", serde_json::json!("other")),
        ] {
            let mut corrupt = original.clone();
            corrupt[field] = value;
            assert!(decode(&serde_json::to_vec(&corrupt).unwrap()).is_err());
        }
    }
}
