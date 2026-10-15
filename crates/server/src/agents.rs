//! Scoped external controllers. Inference is deliberately outside the server.
use super::*;
use sha2::{Digest, Sha256};
use sim_ai::agent::{AgentAction, AgentDecision, AgentObservation, Goal, PROTOCOL_VERSION};

#[derive(Default, Serialize, Deserialize)]
pub(super) struct AgentState {
    pub goals: Vec<Goal>,
    pub connections: BTreeMap<Uuid, Connection>,
    decisions: BTreeMap<Uuid, StoredDecision>,
    history: Vec<serde_json::Value>,
}
impl AgentState {
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty()
            && self.connections.is_empty()
            && self.decisions.is_empty()
            && self.history.is_empty()
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Control {
    Human,
    Deterministic,
    External,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Connection {
    mode: Control,
    generation: u64,
    token_hash: Option<String>,
    timeout_ms: u64,
    fallback: bool,
    work: Option<AgentObservation>,
    last_plan_tick: Option<u64>,
    last_observed_tick: u64,
    #[serde(default)]
    last_evaluation_tick: Option<u64>,
    #[serde(default)]
    last_work_tick: Option<u64>,
    assignments: BTreeMap<Uuid, Uuid>,
    attempts: BTreeMap<Uuid, (u64, sim_core::OrderKind)>,
    receipt_ids: Vec<Uuid>,
}
#[derive(Serialize, Deserialize)]
struct StoredDecision {
    role_id: Uuid,
    decision: AgentDecision,
    result: serde_json::Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configure {
    player_id: Uuid,
    #[serde(rename = "host_player_id")]
    _host_player_id: Uuid,
    mode: Control,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}
fn default_timeout() -> u64 {
    30_000
}
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
pub(super) fn is_agent_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    parts.len() == 6
        && parts[1] == "v1"
        && parts[2] == "games"
        && parts[4] == "agent"
        && matches!(parts[5], "work" | "decisions")
}
pub(super) fn controls(game: &Game, role: Uuid) -> bool {
    game.ai_planner
        .agents
        .connections
        .get(&role)
        .is_some_and(|c| c.mode != Control::Human)
}
// Reserve the most specific AI role even while its external controller is thinking.
pub(super) fn planner_owns(game: &Game, role: Uuid, unit: Uuid) -> bool {
    game.roles
        .values()
        .filter(|r| r.ai_controlled && r.command_units.contains(&unit))
        .min_by_key(|r| (r.command_units.len(), r.id))
        .is_some_and(|r| r.id == role)
}

fn event(game: &mut Game, role: Uuid, kind: &str, details: serde_json::Value) {
    let history = &mut game.ai_planner.agents.history;
    history.push(serde_json::json!({"role_id":role,"tick":game.simulation.tick(),"kind":kind,"details":details}));
    if history.len() > 100 {
        history.remove(0);
    }
}
pub(super) fn invalidate_work(game: &mut Game) {
    for c in game.ai_planner.agents.connections.values_mut() {
        c.work = None;
        c.last_plan_tick = None;
        c.last_evaluation_tick = None;
        c.last_work_tick = None;
    }
}
fn authenticate(
    game: &Game,
    headers: &HeaderMap,
) -> Result<Uuid, (StatusCode, Json<ErrorResponse>)> {
    let supplied = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(hash);
    game.ai_planner
        .agents
        .connections
        .iter()
        .find(|(_, c)| {
            c.mode == Control::External && c.token_hash.is_some() && c.token_hash == supplied
        })
        .map(|(id, _)| *id)
        .ok_or_else(|| {
            api_error(
                StatusCode::UNAUTHORIZED,
                "agent_token_required",
                "Valid scoped agent credential required",
            )
        })
}
pub(super) async fn configure(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    AuthJson(request): AuthJson<Configure>,
) -> ApiResult<serde_json::Value> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    if !teams::can_manage(game, request.player_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "game_control_required",
            "Host or active White Cell required",
        ));
    }
    if !(1_000..=300_000).contains(&request.timeout_ms) {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_timeout",
            "Timeout must be 1000–300000 ms",
        ));
    }
    let role = game.roles.get_mut(&role_id).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "role_not_found",
            "Operational role not found",
        )
    })?;
    leases::release(role_id, &mut role.occupancy);
    role.lease_generation += 1;
    role.ai_controlled = request.mode != Control::Human;
    role.claimable = request.mode == Control::Human;
    let generation = role.lease_generation;
    if role.side == Side::Red {
        if let Some(teams) = &mut game.teams {
            teams.red_control = None;
        }
    }
    let token = if request.mode == Control::External {
        use base64::Engine;
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("system random source");
        Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    } else {
        None
    };
    game.ai_planner.agents.connections.insert(
        role_id,
        Connection {
            mode: request.mode,
            generation,
            token_hash: token.as_deref().map(hash),
            timeout_ms: request.timeout_ms,
            fallback: false,
            work: None,
            last_plan_tick: None,
            last_observed_tick: 0,
            last_evaluation_tick: None,
            last_work_tick: None,
            assignments: BTreeMap::new(),
            attempts: BTreeMap::new(),
            receipt_ids: vec![],
        },
    );
    teams::record_control(
        game,
        request.player_id,
        format!("role {role_id} controller {:?}", request.mode),
    );
    event(
        game,
        role_id,
        "assigned",
        serde_json::json!({"mode":request.mode,"generation":generation}),
    );
    Ok(Json(
        serde_json::json!({"role_id":role_id,"mode":request.mode,"assignment_generation":generation,"token":token}),
    ))
}
pub(super) async fn inspect(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    session: auth::Session,
) -> ApiResult<serde_json::Value> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    if !teams::can_manage(game, session.player_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "game_control_required",
            "Host or active White Cell required",
        ));
    }
    let referee = teams::active_referee(game, session.player_id);
    let roles:Vec<_>=game.roles.values().map(|r| {let c=game.ai_planner.agents.connections.get(&r.id);serde_json::json!({"role_id":r.id,"name":r.name,"side":r.side,"mode":c.map(|c|c.mode).unwrap_or(if r.ai_controlled {Control::Deterministic}else{Control::Human}),"fallback":c.is_some_and(|c|c.fallback),"assignment_generation":c.map(|c|c.generation),"deadline_unix_ms":c.and_then(|c|c.work.as_ref()).map(|w|w.deadline_unix_ms),"observation":if referee {c.and_then(|c|c.work.as_ref())}else{None}})}).collect();
    Ok(Json(
        serde_json::json!({"roles":roles,"history":if referee {game.ai_planner.agents.history.clone()}else{vec![]}}),
    ))
}
pub(super) async fn work(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<serde_json::Value> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    let role = authenticate(game, &headers)?;
    let work = game.ai_planner.agents.connections[&role]
        .work
        .as_ref()
        .filter(|w| {
            game.status == GameStatus::Running
                && !game.simulation.mission_complete()
                && w.deadline_unix_ms > now_ms()
        });
    Ok(Json(serde_json::json!({"work":work})))
}
fn observation(game: &mut Game, role_id: Uuid, now: u64) -> AgentObservation {
    let role = game.roles[&role_id].clone();
    let projection = game
        .simulation
        .projection_for(role.location_unit_id, role.side);
    let connection = game.ai_planner.agents.connections.get(&role_id);
    let receipts = connection
        .into_iter()
        .flat_map(|c| c.receipt_ids.iter().rev().take(32))
        .filter_map(|id| intents::receipt_for_principal(game, role_id, *id, role_id).ok())
        .filter_map(|r| serde_json::to_value(r).ok())
        .collect();
    let authority_requests = game
        .authority_requests
        .values()
        .filter(|r| authority_request_known(r, role_id))
        .filter_map(|r| serde_json::to_value(r).ok())
        .collect();
    let goals = game
        .ai_planner
        .agents
        .goals
        .iter()
        .filter(|g| g.role_id == role_id)
        .cloned()
        .collect();
    let available_actions = game
        .authority
        .policies
        .iter()
        .filter(|p| p.direct_role_ids.contains(&role_id) || p.request_role_ids.contains(&role_id))
        .map(|p| p.action.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    AgentObservation {
        protocol_version: PROTOCOL_VERSION,
        observation_id: game.next_id("agent-observation"),
        assignment_generation: game
            .ai_planner
            .agents
            .connections
            .get(&role_id)
            .map_or(0, |c| c.generation),
        role_id,
        side: role.side,
        controlled_units: role.command_units,
        goal_assignments: game
            .ai_planner
            .agents
            .connections
            .get(&role_id)
            .map(|c| c.assignments.clone())
            .unwrap_or_default(),
        goals,
        projection,
        available_actions,
        authority_requests,
        receipts,
        deadline_unix_ms: now.saturating_add(
            game.ai_planner
                .agents
                .connections
                .get(&role_id)
                .map_or(default_timeout(), |c| c.timeout_ms),
        ),
    }
}
/// `now` is an explicit recorded input; replay never reads the wall clock here.
pub(super) fn advance(game: &mut Game, now: u64) {
    if game.status != GameStatus::Running || game.simulation.mission_complete() {
        invalidate_work(game);
        return;
    }
    let roles: Vec<_> = game
        .ai_planner
        .agents
        .connections
        .iter()
        .filter(|(_, c)| c.mode == Control::External)
        .map(|(r, _)| *r)
        .collect();
    for role in roles {
        let c = &mut game.ai_planner.agents.connections.get_mut(&role).unwrap();
        if c.work.as_ref().is_some_and(|w| w.deadline_unix_ms <= now) {
            c.work = None;
            c.fallback = true;
            event(
                game,
                role,
                "fallback",
                serde_json::json!({"reason":"decision_timeout"}),
            );
        }
        let c = &game.ai_planner.agents.connections[&role];
        if c.work.is_none()
            && c.last_work_tick
                .is_none_or(|t| game.simulation.tick().saturating_sub(t) >= 5)
        {
            let w = observation(game, role, now);
            let c = game.ai_planner.agents.connections.get_mut(&role).unwrap();
            c.last_work_tick = Some(game.simulation.tick());
            c.work = Some(w);
        }
    }
}
fn action(
    game: &mut Game,
    role: Uuid,
    a: &AgentAction,
    namespace: Uuid,
    index: usize,
) -> Result<serde_json::Value, (StatusCode, Json<ErrorResponse>)> {
    match a {
        AgentAction::Order { unit_id, kind } => {
            if !game.roles[&role].command_units.contains(unit_id) {
                return Err(api_error(
                    StatusCode::FORBIDDEN,
                    "unit_scope",
                    "Unit outside assigned role",
                ));
            }
            if ai_orders::order_is_pending(game, *unit_id)
                || game.authority_requests.values().any(|r| {
                    r.target_unit_id == *unit_id
                        && matches!(
                            r.status,
                            AuthorityRequestStatus::InTransit { .. }
                                | AuthorityRequestStatus::PendingHuman { .. }
                                | AuthorityRequestStatus::WaitingVacant { .. }
                                | AuthorityRequestStatus::PendingExternal { .. }
                        )
                })
            {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    "order_pending",
                    "Unit already has an order in transit",
                ));
            }
            let intent_id = Uuid::new_v5(&namespace, index.to_string().as_bytes());
            let intent = PlayerIntent {
                intent_id,
                issuer_role: role,
                target: *unit_id,
                kind: kind.clone(),
                requested_tick: game.simulation.tick().saturating_add(1),
            };
            let result = intents::submit_validated_intent(game, role, role, intent)?;
            let c = game.ai_planner.agents.connections.get_mut(&role).unwrap();
            c.receipt_ids.push(intent_id);
            if c.receipt_ids.len() > 64 {
                c.receipt_ids.remove(0);
            }
            Ok(serde_json::json!({"intent_id":intent_id,"submission":result}))
        }
        AgentAction::SelectGoal { goal_id, unit_ids } => {
            if !game
                .ai_planner
                .agents
                .goals
                .iter()
                .any(|g| g.id == *goal_id && g.role_id == role)
                || unit_ids
                    .iter()
                    .any(|id| !game.roles[&role].command_units.contains(id))
            {
                return Err(api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_goal_assignment",
                    "Goal and units must belong to assigned role",
                ));
            }
            let c = game.ai_planner.agents.connections.get_mut(&role).unwrap();
            for id in unit_ids {
                c.assignments.insert(*id, *goal_id);
            }
            Ok(serde_json::json!({"goal_id":goal_id,"unit_ids":unit_ids}))
        }
        AgentAction::AuthorityDecision {
            request_id,
            approve,
        } => decide_validated_authority(game, role, *request_id, *approve)
            .map(|r| serde_json::json!({"request_id":r.id})),
    }
}
pub(super) async fn decide(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(decision): Json<AgentDecision>,
) -> ApiResult<serde_json::Value> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    let role = authenticate(game, &headers)?;
    let c = &game.ai_planner.agents.connections[&role];
    if decision.protocol_version != PROTOCOL_VERSION
        || decision.assignment_generation != c.generation
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "agent_generation",
            "Unsupported protocol or obsolete assignment",
        ));
    }
    if let Some(old) = game.ai_planner.agents.decisions.get(&decision.decision_id) {
        return if old.role_id == role && old.decision == decision {
            Ok(Json(old.result.clone()))
        } else {
            Err(api_error(
                StatusCode::CONFLICT,
                "decision_conflict",
                "Decision ID already used",
            ))
        };
    }
    if game.status != GameStatus::Running
        || game.simulation.mission_complete()
        || !c.work.as_ref().is_some_and(|w| {
            w.observation_id == decision.observation_id && w.deadline_unix_ms > now_ms()
        })
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "decision_closed",
            "Decision request is closed",
        ));
    }
    if decision.actions.len() > 32
        || decision.explanation.len() > 2048
        || decision.model.as_ref().is_some_and(|s| s.len() > 200)
    {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "decision_size",
            "Decision exceeds limits",
        ));
    }
    let observed = c.work.as_ref().unwrap().clone();
    let mut results = Vec::new();
    for (index, a) in decision.actions.iter().enumerate() {
        let visible = match a {
            AgentAction::AuthorityDecision { request_id, .. } => observed
                .authority_requests
                .iter()
                .any(|r| r["id"] == request_id.to_string()),
            AgentAction::Order {
                kind: sim_core::OrderKind::Engage { track_id },
                ..
            } => observed
                .projection
                .tracks
                .iter()
                .any(|t| t.track_id == *track_id),
            _ => true,
        };
        let outcome = if visible {
            action(game, role, a, decision.decision_id, index)
        } else {
            Err(api_error(
                StatusCode::FORBIDDEN,
                "not_observed",
                "Action refers to information absent from this observation",
            ))
        };
        results.push(match outcome {
            Ok(result) => {
                if let AgentAction::Order { unit_id, .. } = a {
                    game.ai_planner
                        .agents
                        .connections
                        .get_mut(&role)
                        .unwrap()
                        .assignments
                        .remove(unit_id);
                }
                serde_json::json!({"index":index,"accepted":true,"result":result})
            }
            Err((_, Json(error))) => {
                serde_json::json!({"index":index,"accepted":false,"error":error})
            }
        });
    }
    let result = serde_json::json!({"decision_id":decision.decision_id,"results":results});
    let c = game.ai_planner.agents.connections.get_mut(&role).unwrap();
    c.work = None;
    c.fallback = false;
    event(
        game,
        role,
        "decision",
        serde_json::json!({"decision_id":decision.decision_id,"actions":decision.actions,"explanation":decision.explanation,"model":decision.model,"result":result}),
    );
    game.ai_planner.agents.decisions.insert(
        decision.decision_id,
        StoredDecision {
            role_id: role,
            decision,
            result: result.clone(),
        },
    );
    Ok(Json(result))
}

pub(super) fn process_policies(game: &mut Game) {
    let candidates: Vec<_> = game
        .roles
        .values()
        .filter(|r| {
            r.ai_controlled
                && game
                    .ai_planner
                    .agents
                    .goals
                    .iter()
                    .any(|g| g.role_id == r.id)
        })
        .map(|r| r.id)
        .collect();
    for role in candidates {
        if !game.ai_planner.agents.connections.contains_key(&role) {
            let generation = game.roles[&role].lease_generation;
            game.ai_planner.agents.connections.insert(
                role,
                Connection {
                    mode: Control::Deterministic,
                    generation,
                    token_hash: None,
                    timeout_ms: default_timeout(),
                    fallback: false,
                    work: None,
                    last_plan_tick: None,
                    last_observed_tick: 0,
                    last_evaluation_tick: None,
                    last_work_tick: None,
                    assignments: BTreeMap::new(),
                    attempts: BTreeMap::new(),
                    receipt_ids: vec![],
                },
            );
        }
    }
    let mut roles: Vec<_> = game
        .ai_planner
        .agents
        .connections
        .iter()
        .filter(|(_, c)| {
            c.mode == Control::Deterministic
                || (c.mode == Control::External && (c.fallback || !c.assignments.is_empty()))
        })
        .map(|(r, _)| *r)
        .collect();
    roles.sort_by_key(|id| (game.roles[id].command_units.len(), *id));
    let mut assigned = BTreeSet::new();
    for role in roles {
        let tick = game.simulation.tick();
        let c = game.ai_planner.agents.connections.get_mut(&role).unwrap();
        if c.last_evaluation_tick == Some(tick) {
            continue;
        }
        c.last_evaluation_tick = Some(tick);
        let mut obs = observation(game, role, 0);
        let c = &game.ai_planner.agents.connections[&role];
        obs.controlled_units.retain(|id| {
            planner_owns(game, role, *id)
                && (c.mode != Control::External || c.fallback || c.assignments.contains_key(id))
                && assigned.insert(*id)
        });
        let observed = obs
            .projection
            .tracks
            .iter()
            .map(|t| t.observed_tick)
            .max()
            .unwrap_or(0);
        let c = &game.ai_planner.agents.connections[&role];
        if c.last_plan_tick.is_some_and(|t| {
            obs.projection.tick == t
                || (obs.projection.tick.saturating_sub(t) < 5 && observed <= c.last_observed_tick)
        }) {
            continue;
        }
        let actions = sim_ai::agent::defensive_actions(&obs, &c.assignments, game.network_seed);
        let c = game.ai_planner.agents.connections.get_mut(&role).unwrap();
        c.last_plan_tick = Some(obs.projection.tick);
        c.last_observed_tick = observed;
        let mut outcomes = Vec::new();
        for (i, a) in actions.into_iter().enumerate() {
            if let AgentAction::Order { unit_id, ref kind } = a {
                if game.ai_planner.agents.connections[&role]
                    .attempts
                    .get(&unit_id)
                    .is_some_and(|(tick, last)| {
                        last == kind && obs.projection.tick.saturating_sub(*tick) < 10
                    })
                {
                    continue;
                }
                game.ai_planner
                    .agents
                    .connections
                    .get_mut(&role)
                    .unwrap()
                    .attempts
                    .insert(unit_id, (obs.projection.tick, kind.clone()));
            }
            let outcome = match action(game, role, &a, obs.observation_id, i) {
                Ok(result) => serde_json::json!({"action":a,"accepted":true,"result":result}),
                Err((_, Json(error))) => {
                    serde_json::json!({"action":a,"accepted":false,"error":error})
                }
            };
            outcomes.push(outcome);
        }
        if !outcomes.is_empty() {
            event(
                game,
                role,
                "policy_decision",
                serde_json::json!({"policy":"defense.v1","goals":obs.goals.iter().map(|g|g.id).collect::<Vec<_>>(),"outcomes":outcomes}),
            );
        }
    }
}

pub(super) fn authority_changed(game: &mut Game) {
    invalidate_work(game);
    for (id, c) in &mut game.ai_planner.agents.connections {
        if let Some(role) = game.roles.get_mut(id) {
            role.ai_controlled = c.mode != Control::Human;
            role.claimable = c.mode == Control::Human;
            if c.mode != Control::Human {
                role.lease_generation += 1;
                c.generation = role.lease_generation;
            }
            c.assignments
                .retain(|unit, _| role.command_units.contains(unit));
        }
    }
}

#[cfg(test)]
mod tests;
