mod credential_cookie;
mod space_catalog;

use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header::SET_COOKIE, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use credential_cookie::{CredentialCookie, RememberedCredentials};
use serde::{Deserialize, Serialize};
use sim_ai::choose_patrol_intent;
use sim_core::{
    AuthorityDefinition, AuthorityPolicy, AuthorityRoleKind, AuthorizationRecord, AuthorizedIntent,
    PlayerIntent, RoleProjection, Side, Simulation,
};
use sim_scenario::{global_crisis_scenario, Scenario};
use space_catalog::{SpaceCatalogService, SpaceCatalogSnapshot, SpaceCatalogStatus};
use tokio::sync::RwLock;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    games: Arc<RwLock<BTreeMap<Uuid, Game>>>,
    scenarios: Arc<BTreeMap<String, Scenario>>,
    space_catalog: SpaceCatalogService,
    admin_token: Arc<Option<String>>,
    credential_cookie: CredentialCookie,
}

struct Game {
    id: Uuid,
    title: String,
    host: Uuid,
    status: GameStatus,
    simulation: Simulation,
    roles: BTreeMap<Uuid, Role>,
    authority: AuthorityDefinition,
    authority_requests: BTreeMap<Uuid, AuthorityRequest>,
    authority_events: Vec<AuthorityEvent>,
    unit_ids: BTreeSet<Uuid>,
    space_catalog_checksum: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GameStatus {
    Lobby,
    Running,
    Paused,
}

#[derive(Debug, Clone)]
struct Role {
    id: Uuid,
    name: String,
    side: Side,
    kind: AuthorityRoleKind,
    location_unit_id: Uuid,
    command_units: Vec<Uuid>,
    claimable: bool,
    owner: Option<Uuid>,
    ai_controlled: bool,
    lease_generation: u64,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}
#[derive(Serialize)]
struct ScenarioSummary {
    id: String,
    title: String,
    description: String,
    version: u32,
    authored_entity_count: usize,
    role_count: usize,
    requires_space_catalog: bool,
}
#[derive(Serialize)]
struct GameSummary {
    id: Uuid,
    title: String,
    status: GameStatus,
    host_player_id: Uuid,
    player_roles_available: usize,
}
#[derive(Serialize)]
struct RoleSummary {
    id: Uuid,
    name: String,
    side: Side,
    kind: AuthorityRoleKind,
    location_unit_id: Uuid,
    command_units: Vec<Uuid>,
    held: bool,
    ai_controlled: bool,
    lease_generation: u64,
}
#[derive(Deserialize)]
struct CreateGameRequest {
    scenario_id: String,
    title: Option<String>,
    host_player_id: Uuid,
}
#[derive(Serialize)]
struct CreateGameResponse {
    game: GameSummary,
    host_player_id: Uuid,
}
#[derive(Deserialize)]
struct JoinRequest {
    display_name: String,
}
#[derive(Serialize)]
struct JoinResponse {
    player_id: Uuid,
    display_name: String,
}
#[derive(Deserialize)]
struct ClaimRoleRequest {
    player_id: Uuid,
}
#[derive(Deserialize)]
struct GameControlRequest {
    player_id: Uuid,
}
#[derive(Deserialize)]
struct SubmitIntentRequest {
    player_id: Uuid,
    lease_generation: u64,
    intent: PlayerIntent,
}
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SubmissionOutcome {
    Queued { intent_id: Uuid },
    PendingAuthority { request_id: Uuid },
}
#[derive(Debug, Clone, Serialize)]
struct AuthorityDecisionRecord {
    role_id: Uuid,
    approved: bool,
    automatic: bool,
    tick: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum AuthorityRequestStatus {
    PendingHuman {
        role_id: Uuid,
    },
    WaitingVacant {
        role_id: Uuid,
        resolves_at_tick: u64,
    },
    Approved,
    ApprovedNoExecutor,
    Denied {
        role_id: Uuid,
    },
    BlockedComms,
}
#[derive(Debug, Clone, Serialize)]
struct AuthorityRequest {
    id: Uuid,
    action: String,
    target_unit_id: Uuid,
    requester_role_id: Uuid,
    policy: AuthorityPolicy,
    policy_version: u64,
    current_step: usize,
    created_tick: u64,
    summary: String,
    status: AuthorityRequestStatus,
    decisions: Vec<AuthorityDecisionRecord>,
    #[serde(skip)]
    intent: Option<PlayerIntent>,
}
#[derive(Debug, Clone, Serialize)]
struct AuthorityEvent {
    tick: u64,
    kind: String,
    detail: String,
}
#[derive(Deserialize)]
struct AuthorityQuery {
    player_id: Uuid,
}
#[derive(Deserialize)]
struct AuthorityRequestsQuery {
    player_id: Uuid,
    role_id: Option<Uuid>,
}
#[derive(Deserialize)]
struct UpdateAuthorityRequest {
    player_id: Uuid,
    expected_version: u64,
    definition: AuthorityDefinition,
}
#[derive(Deserialize)]
struct CreateAuthorityRequest {
    player_id: Uuid,
    lease_generation: u64,
    action: String,
    target_unit_id: Uuid,
    summary: String,
}
#[derive(Deserialize)]
struct DecideAuthorityRequest {
    player_id: Uuid,
    lease_generation: u64,
    decision: AuthorityDecision,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AuthorityDecision {
    Approve,
    Deny,
}
#[derive(Deserialize)]
struct ProjectionQuery {
    player_id: Uuid,
    role_id: Uuid,
}
#[derive(Deserialize)]
struct SpaceTrackConnectRequest {
    username: String,
    password: String,
    #[serde(default)]
    remember: bool,
}
#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: &'static str,
    error: String,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;
type CookieApiResult<T> = Result<(HeaderMap, Json<T>), (StatusCode, Json<ErrorResponse>)>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let scenario = global_crisis_scenario();
    scenario.validate()?;
    let admin_token = std::env::var("ADMIN_SETUP_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty());
    let state = AppState {
        games: Arc::new(RwLock::new(BTreeMap::new())),
        scenarios: Arc::new(BTreeMap::from([(scenario.id.clone(), scenario)])),
        space_catalog: SpaceCatalogService::load().await?,
        admin_token: Arc::new(admin_token),
        credential_cookie: CredentialCookie::load().await?,
    };
    tokio::spawn(run_simulation_loop(state.clone()));
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/scenarios", get(list_scenarios))
        .route("/v1/games", get(list_games).post(create_game))
        .route("/v1/games/{game_id}/join", post(join_game))
        .route("/v1/games/{game_id}/roles", get(list_roles))
        .route(
            "/v1/games/{game_id}/authority",
            get(get_authority).put(update_authority),
        )
        .route(
            "/v1/games/{game_id}/authority/requests",
            get(list_authority_requests),
        )
        .route(
            "/v1/games/{game_id}/roles/{role_id}/authority-requests",
            post(create_authority_request),
        )
        .route(
            "/v1/games/{game_id}/roles/{role_id}/authority-requests/{request_id}/decision",
            post(decide_authority_request),
        )
        .route(
            "/v1/games/{game_id}/roles/{role_id}/claim",
            post(claim_role),
        )
        .route("/v1/games/{game_id}/start", post(start_game))
        .route("/v1/games/{game_id}/pause", post(pause_game))
        .route(
            "/v1/games/{game_id}/roles/{role_id}/intent",
            post(submit_intent),
        )
        .route("/v1/games/{game_id}/state", get(get_projection))
        .route("/v1/games/{game_id}/stream", get(stream_projection))
        .route("/v1/games/{game_id}/space-catalog", get(game_space_catalog))
        .route(
            "/v1/settings/space-catalog/status",
            get(space_catalog_status),
        )
        .route("/v1/admin/space-track/connect", post(connect_space_track))
        .route("/v1/admin/space-catalog/sync", post(sync_space_catalog))
        .route(
            "/v1/settings/space-track/credentials",
            post(restore_space_track).delete(forget_space_track),
        )
        .layer(CompressionLayer::new())
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::mirror_request())
                .allow_methods(AllowMethods::mirror_request())
                .allow_headers(AllowHeaders::mirror_request())
                .allow_credentials(true),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    let address: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8000".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    println!("World At War server listening on http://{address}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn list_scenarios(State(state): State<AppState>) -> Json<Vec<ScenarioSummary>> {
    Json(
        state
            .scenarios
            .values()
            .map(|scenario| ScenarioSummary {
                id: scenario.id.clone(),
                title: scenario.title.clone(),
                description: scenario.description.clone(),
                version: scenario.version,
                authored_entity_count: scenario.units.len(),
                role_count: scenario.authority.roles.len(),
                requires_space_catalog: scenario.requires_space_catalog,
            })
            .collect(),
    )
}

async fn list_games(State(state): State<AppState>) -> Json<Vec<GameSummary>> {
    Json(
        state
            .games
            .read()
            .await
            .values()
            .map(game_summary)
            .collect(),
    )
}

async fn create_game(
    State(state): State<AppState>,
    Json(request): Json<CreateGameRequest>,
) -> ApiResult<CreateGameResponse> {
    let scenario = state.scenarios.get(&request.scenario_id).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "scenario_not_found",
            "scenario not found",
        )
    })?;
    let catalog = state.space_catalog.status().await;
    if scenario.requires_space_catalog && !catalog.usable {
        return Err(api_error(
            StatusCode::CONFLICT,
            "space_catalog_unavailable",
            "connect Space-Track and synchronize a current catalog before creating this scenario",
        ));
    }
    let checksum = catalog.checksum.ok_or_else(|| {
        api_error(
            StatusCode::CONFLICT,
            "space_catalog_unavailable",
            "space catalog snapshot is missing",
        )
    })?;
    let simulation = scenario.spawn().map_err(|error| {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_scenario",
            error.to_string(),
        )
    })?;
    let authority = scenario.authority.clone();
    let roles = authority
        .roles
        .iter()
        .map(|template| {
            (
                template.id,
                Role {
                    id: template.id,
                    name: template.name.clone(),
                    side: template.side,
                    kind: template.kind,
                    location_unit_id: template.location_unit_id,
                    command_units: authority.controlled_units(template.id),
                    claimable: template.claimable,
                    owner: None,
                    ai_controlled: template.ai_controlled,
                    lease_generation: 0,
                },
            )
        })
        .collect();
    let game_id = Uuid::new_v4();
    let game = Game {
        id: game_id,
        title: request
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| scenario.title.clone()),
        host: request.host_player_id,
        status: GameStatus::Lobby,
        simulation,
        roles,
        authority,
        authority_requests: BTreeMap::new(),
        authority_events: Vec::new(),
        unit_ids: scenario.units.iter().map(|unit| unit.id).collect(),
        space_catalog_checksum: checksum,
    };
    let summary = game_summary(&game);
    state.games.write().await.insert(game_id, game);
    Ok(Json(CreateGameResponse {
        game: summary,
        host_player_id: request.host_player_id,
    }))
}

async fn join_game(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<JoinRequest>,
) -> ApiResult<JoinResponse> {
    if request.display_name.trim().is_empty() {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "display_name_required",
            "display name is required",
        ));
    }
    if !state.games.read().await.contains_key(&game_id) {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "game_not_found",
            "game not found",
        ));
    }
    Ok(Json(JoinResponse {
        player_id: Uuid::new_v4(),
        display_name: request.display_name,
    }))
}

async fn list_roles(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
) -> ApiResult<Vec<RoleSummary>> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    Ok(Json(game.roles.values().map(role_summary).collect()))
}

async fn claim_role(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<ClaimRoleRequest>,
) -> ApiResult<RoleSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    let role = game
        .roles
        .get_mut(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "role not found"))?;
    if role.ai_controlled || !role.claimable {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "ai_role",
            "this role is controlled by scenario AI",
        ));
    }
    if role.owner.is_some() && role.owner != Some(request.player_id) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "role_held",
            "role is already held",
        ));
    }
    role.owner = Some(request.player_id);
    role.lease_generation += 1;
    Ok(Json(role_summary(role)))
}

async fn start_game(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<GameControlRequest>,
) -> ApiResult<GameSummary> {
    set_game_status(state, game_id, request.player_id, GameStatus::Running).await
}
async fn pause_game(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<GameControlRequest>,
) -> ApiResult<GameSummary> {
    set_game_status(state, game_id, request.player_id, GameStatus::Paused).await
}
async fn set_game_status(
    state: AppState,
    game_id: Uuid,
    player: Uuid,
    status: GameStatus,
) -> ApiResult<GameSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    if game.host != player {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "host_required",
            "only the host can control game state",
        ));
    }
    game.status = status;
    Ok(Json(game_summary(game)))
}

async fn submit_intent(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<SubmitIntentRequest>,
) -> ApiResult<SubmissionOutcome> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    if game.status != GameStatus::Running {
        return Err(api_error(
            StatusCode::CONFLICT,
            "game_not_running",
            "game is not running",
        ));
    }
    let role = game
        .roles
        .get(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "role not found"))?;
    if role.owner != Some(request.player_id) || role.lease_generation != request.lease_generation {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "invalid_role_lease",
            "invalid role lease",
        ));
    }
    if request.intent.issuer_role != role_id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "issuer_role",
            "intent issuer does not match the held role",
        ));
    }
    let action = request.intent.kind.action_key().to_string();
    let target = request.intent.target;
    let outcome = submit_authority_action(
        game,
        role_id,
        action,
        target,
        String::new(),
        Some(request.intent),
    )?;
    Ok(Json(outcome))
}

async fn get_authority(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<AuthorityQuery>,
) -> ApiResult<AuthorityDefinition> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    require_game_participant(game, query.player_id)?;
    Ok(Json(game.authority.clone()))
}

async fn update_authority(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Json(mut request): Json<UpdateAuthorityRequest>,
) -> ApiResult<AuthorityDefinition> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    if game.host != request.player_id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "host_required",
            "only the host can edit authorities",
        ));
    }
    if request.expected_version != game.authority.version {
        return Err(api_error(
            StatusCode::CONFLICT,
            "authority_version_conflict",
            format!(
                "authority definition is now version {}",
                game.authority.version
            ),
        ));
    }
    request
        .definition
        .validate(&game.unit_ids)
        .map_err(|error| api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_authority", error))?;
    let new_ids: BTreeSet<_> = request
        .definition
        .roles
        .iter()
        .map(|role| role.id)
        .collect();
    for old in game
        .roles
        .values()
        .filter(|role| role.owner.is_some() || role.ai_controlled)
    {
        let Some(new_role) = request
            .definition
            .roles
            .iter()
            .find(|role| role.id == old.id)
        else {
            return Err(api_error(
                StatusCode::CONFLICT,
                "occupied_role_removed",
                format!("occupied or AI role {} cannot be removed", old.name),
            ));
        };
        if new_role.side != old.side {
            return Err(api_error(
                StatusCode::CONFLICT,
                "occupied_role_side_changed",
                format!("occupied role {} cannot change side", old.name),
            ));
        }
    }
    request.definition.version = game.authority.version + 1;
    let mut roles = BTreeMap::new();
    for definition in &request.definition.roles {
        let previous = game.roles.get(&definition.id);
        roles.insert(
            definition.id,
            Role {
                id: definition.id,
                name: definition.name.clone(),
                side: definition.side,
                kind: definition.kind,
                location_unit_id: definition.location_unit_id,
                command_units: request.definition.controlled_units(definition.id),
                claimable: definition.claimable,
                owner: previous.and_then(|role| role.owner),
                ai_controlled: definition.ai_controlled,
                lease_generation: previous.map_or(0, |role| role.lease_generation),
            },
        );
    }
    debug_assert_eq!(roles.len(), new_ids.len());
    game.roles = roles;
    game.authority = request.definition;
    game.authority_events.push(AuthorityEvent {
        tick: game.simulation.tick(),
        kind: "authority_updated".into(),
        detail: format!("version {} saved by host", game.authority.version),
    });
    Ok(Json(game.authority.clone()))
}

async fn list_authority_requests(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<AuthorityRequestsQuery>,
) -> ApiResult<Vec<AuthorityRequest>> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    require_game_participant(game, query.player_id)?;
    if game.host == query.player_id && query.role_id.is_none() {
        return Ok(Json(game.authority_requests.values().cloned().collect()));
    }
    let role_id = query.role_id.ok_or_else(|| {
        api_error(
            StatusCode::BAD_REQUEST,
            "role_required",
            "role_id is required for non-host players",
        )
    })?;
    let role = game
        .roles
        .get(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "role not found"))?;
    if role.owner != Some(query.player_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "role_not_held",
            "role is not held by this player",
        ));
    }
    Ok(Json(
        game.authority_requests
            .values()
            .filter(|request| {
                request.requester_role_id == role_id
                    || current_decision_role(request) == Some(role_id)
                    || request.policy.notify_role_ids.contains(&role_id)
            })
            .cloned()
            .collect(),
    ))
}

async fn create_authority_request(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<CreateAuthorityRequest>,
) -> ApiResult<SubmissionOutcome> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    if request.summary.chars().count() > 500 {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "summary_too_long",
            "request summary is limited to 500 characters",
        ));
    }
    let outcome = submit_authority_action(
        game,
        role_id,
        request.action,
        request.target_unit_id,
        request.summary,
        None,
    )?;
    Ok(Json(outcome))
}

async fn decide_authority_request(
    Path((game_id, role_id, request_id)): Path<(Uuid, Uuid, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<DecideAuthorityRequest>,
) -> ApiResult<AuthorityRequest> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    let mut authority_request = game.authority_requests.remove(&request_id).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "authority_request_not_found",
            "authority request not found",
        )
    })?;
    if current_decision_role(&authority_request) != Some(role_id)
        || !matches!(
            authority_request.status,
            AuthorityRequestStatus::PendingHuman { .. }
        )
    {
        game.authority_requests
            .insert(request_id, authority_request);
        return Err(api_error(
            StatusCode::CONFLICT,
            "decision_not_available",
            "this role cannot decide the request now",
        ));
    }
    advance_authority_request(
        game,
        &mut authority_request,
        matches!(request.decision, AuthorityDecision::Approve),
        false,
    );
    let response = authority_request.clone();
    game.authority_requests
        .insert(request_id, authority_request);
    Ok(Json(response))
}

trait CommunicationsGate {
    fn reachable(&self, _from_role: Uuid, _to: Uuid) -> bool;
}
struct AlwaysReachable;
impl CommunicationsGate for AlwaysReachable {
    fn reachable(&self, _: Uuid, _: Uuid) -> bool {
        true
    }
}

fn submit_authority_action(
    game: &mut Game,
    role_id: Uuid,
    action: String,
    target: Uuid,
    summary: String,
    intent: Option<PlayerIntent>,
) -> Result<SubmissionOutcome, (StatusCode, Json<ErrorResponse>)> {
    let policy = game
        .authority
        .policy_for(&action, target)
        .cloned()
        .ok_or_else(|| {
            api_error(
                StatusCode::FORBIDDEN,
                "authority_not_defined",
                "no authority policy covers this action and target",
            )
        })?;
    if policy.direct_role_ids.contains(&role_id)
        && game.authority.role_is_in_unit_chain(role_id, target)
    {
        if !AlwaysReachable.reachable(role_id, target) {
            return Err(api_error(
                StatusCode::CONFLICT,
                "blocked_comms",
                "no communications route to the target",
            ));
        }
        if let Some(intent) = intent {
            let intent_id = intent.intent_id;
            game.simulation.queue_authorized_intent(AuthorizedIntent {
                intent,
                authorization: AuthorizationRecord {
                    policy_id: policy.id,
                    policy_version: game.authority.version,
                    requester_role_id: role_id,
                    granting_role_id: role_id,
                    request_id: None,
                },
            });
            return Ok(SubmissionOutcome::Queued { intent_id });
        }
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "direct_action_requires_intent",
            "this action is directly executable and requires an order payload",
        ));
    }
    if !policy.request_role_ids.contains(&role_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "request_not_permitted",
            "role may neither issue nor request this action",
        ));
    }
    let Some(first_step) = policy.decision_steps.first() else {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "missing_decision_step",
            "authority policy has no decision step",
        ));
    };
    if !AlwaysReachable.reachable(role_id, first_step.role_id) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "blocked_comms",
            "no communications route to the deciding role",
        ));
    }
    let request_id = Uuid::new_v4();
    let tick = game.simulation.tick();
    let status = status_for_decision_role(game, first_step, tick);
    game.authority_requests.insert(
        request_id,
        AuthorityRequest {
            id: request_id,
            action,
            target_unit_id: target,
            requester_role_id: role_id,
            policy,
            policy_version: game.authority.version,
            current_step: 0,
            created_tick: tick,
            summary,
            status,
            decisions: Vec::new(),
            intent,
        },
    );
    game.authority_events.push(AuthorityEvent {
        tick,
        kind: "authority_requested".into(),
        detail: format!("request {request_id} created"),
    });
    Ok(SubmissionOutcome::PendingAuthority { request_id })
}

fn validate_role_lease(
    game: &Game,
    role_id: Uuid,
    player_id: Uuid,
    lease_generation: u64,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let role = game
        .roles
        .get(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "role not found"))?;
    if role.owner != Some(player_id) || role.lease_generation != lease_generation {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "invalid_role_lease",
            "invalid role lease",
        ));
    }
    Ok(())
}

fn require_game_participant(
    game: &Game,
    player_id: Uuid,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if game.host == player_id
        || game
            .roles
            .values()
            .any(|role| role.owner == Some(player_id))
    {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            "game_participant_required",
            "player is not participating in this game",
        ))
    }
}

fn current_decision_role(request: &AuthorityRequest) -> Option<Uuid> {
    request
        .policy
        .decision_steps
        .get(request.current_step)
        .map(|step| step.role_id)
}

fn status_for_decision_role(
    game: &Game,
    step: &sim_core::AuthorityDecisionStep,
    tick: u64,
) -> AuthorityRequestStatus {
    if game
        .roles
        .get(&step.role_id)
        .and_then(|role| role.owner)
        .is_some()
    {
        AuthorityRequestStatus::PendingHuman {
            role_id: step.role_id,
        }
    } else {
        AuthorityRequestStatus::WaitingVacant {
            role_id: step.role_id,
            resolves_at_tick: tick.saturating_add(step.vacant_delay_ticks),
        }
    }
}

fn advance_authority_request(
    game: &mut Game,
    request: &mut AuthorityRequest,
    approved: bool,
    automatic: bool,
) {
    let Some(role_id) = current_decision_role(request) else {
        return;
    };
    let tick = game.simulation.tick();
    request.decisions.push(AuthorityDecisionRecord {
        role_id,
        approved,
        automatic,
        tick,
    });
    if !approved {
        request.status = AuthorityRequestStatus::Denied { role_id };
        return;
    }
    request.current_step += 1;
    if let Some(step) = request.policy.decision_steps.get(request.current_step) {
        if !AlwaysReachable.reachable(role_id, step.role_id) {
            request.status = AuthorityRequestStatus::BlockedComms;
        } else {
            request.status = status_for_decision_role(game, step, tick);
        }
        return;
    }
    if !request.policy.executable || request.intent.is_none() {
        request.status = AuthorityRequestStatus::ApprovedNoExecutor;
        return;
    }
    if !AlwaysReachable.reachable(role_id, request.target_unit_id) {
        request.status = AuthorityRequestStatus::BlockedComms;
        return;
    }
    let intent = request.intent.take().expect("checked above");
    game.simulation.queue_authorized_intent(AuthorizedIntent {
        intent,
        authorization: AuthorizationRecord {
            policy_id: request.policy.id,
            policy_version: request.policy_version,
            requester_role_id: request.requester_role_id,
            granting_role_id: role_id,
            request_id: Some(request.id),
        },
    });
    request.status = AuthorityRequestStatus::Approved;
}

fn process_vacant_authority_requests(game: &mut Game) {
    let ids: Vec<_> = game.authority_requests.keys().copied().collect();
    for id in ids {
        let Some(mut request) = game.authority_requests.remove(&id) else {
            continue;
        };
        let Some(step) = request
            .policy
            .decision_steps
            .get(request.current_step)
            .cloned()
        else {
            game.authority_requests.insert(id, request);
            continue;
        };
        let occupied = game
            .roles
            .get(&step.role_id)
            .and_then(|role| role.owner)
            .is_some();
        match request.status {
            AuthorityRequestStatus::WaitingVacant {
                resolves_at_tick: _,
                ..
            } if occupied => {
                request.status = AuthorityRequestStatus::PendingHuman {
                    role_id: step.role_id,
                };
            }
            AuthorityRequestStatus::WaitingVacant {
                resolves_at_tick, ..
            } if game.simulation.tick() >= resolves_at_tick => {
                let sample = ((request.id.as_u128()
                    ^ request.policy.id.as_u128()
                    ^ request.policy_version as u128)
                    % 10_000) as u16;
                advance_authority_request(
                    game,
                    &mut request,
                    sample < step.approve_probability_bps,
                    true,
                );
            }
            AuthorityRequestStatus::PendingHuman { .. } if !occupied => {
                request.status = AuthorityRequestStatus::WaitingVacant {
                    role_id: step.role_id,
                    resolves_at_tick: game
                        .simulation
                        .tick()
                        .saturating_add(step.vacant_delay_ticks),
                };
            }
            _ => {}
        }
        game.authority_requests.insert(id, request);
    }
}

async fn get_projection(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ProjectionQuery>,
) -> ApiResult<RoleProjection> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    let role = authorized_role(game, &query)?;
    Ok(Json(
        game.simulation
            .projection_for(role.location_unit_id, role.side),
    ))
}

async fn game_space_catalog(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ProjectionQuery>,
) -> ApiResult<SpaceCatalogSnapshot> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    authorized_role(game, &query)?;
    let snapshot = state
        .space_catalog
        .snapshot(&game.space_catalog_checksum)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::GONE,
                "catalog_snapshot_missing",
                "the game's pinned space catalog is no longer available",
            )
        })?;
    Ok(Json(snapshot))
}

fn authorized_role<'a>(
    game: &'a Game,
    query: &ProjectionQuery,
) -> Result<&'a Role, (StatusCode, Json<ErrorResponse>)> {
    let role = game
        .roles
        .get(&query.role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "role not found"))?;
    if role.owner != Some(query.player_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "role_not_held",
            "role is not held by this player",
        ));
    }
    Ok(role)
}

async fn space_catalog_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<SpaceCatalogStatus> {
    let mut status = state.space_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    status.remembered_credentials = remembered_credentials(&state, &headers).is_some();
    Json(status)
}
async fn connect_space_track(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SpaceTrackConnectRequest>,
) -> CookieApiResult<SpaceCatalogStatus> {
    require_admin(&state, &headers)?;
    let remembered = RememberedCredentials {
        username: request.username.clone(),
        password: request.password.clone(),
    };
    let mut status = state
        .space_catalog
        .configure_and_sync(request.username, request.password)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::BAD_GATEWAY,
                "space_track_error",
                error.to_string(),
            )
        })?;
    status.setup_auth_required = state.admin_token.is_some();
    status.remembered_credentials = request.remember;
    let cookie = if request.remember {
        let value = state.credential_cookie.seal(&remembered).map_err(|error| {
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "credential_cookie_error",
                error.to_string(),
            )
        })?;
        state.credential_cookie.set_header(&value)
    } else {
        state.credential_cookie.clear_header()
    };
    Ok((set_cookie_headers(cookie)?, Json(status)))
}

async fn restore_space_track(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<SpaceCatalogStatus> {
    let credentials = remembered_credentials(&state, &headers).ok_or_else(|| {
        api_error(
            StatusCode::UNAUTHORIZED,
            "remembered_credentials_missing",
            "no valid saved Space-Track credentials were found",
        )
    })?;
    let mut status = state
        .space_catalog
        .restore_credentials(credentials.username, credentials.password)
        .await
        .map_err(|error| {
            api_error(
                StatusCode::BAD_GATEWAY,
                "space_track_restore_error",
                error.to_string(),
            )
        })?;
    status.setup_auth_required = state.admin_token.is_some();
    status.remembered_credentials = true;
    Ok(Json(status))
}

async fn forget_space_track(State(state): State<AppState>) -> CookieApiResult<SpaceCatalogStatus> {
    state.space_catalog.clear_credentials().await;
    let mut status = state.space_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    status.remembered_credentials = false;
    Ok((
        set_cookie_headers(state.credential_cookie.clear_header())?,
        Json(status),
    ))
}
async fn sync_space_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<SpaceCatalogStatus> {
    require_admin(&state, &headers)?;
    state.space_catalog.sync(false).await.map_err(|error| {
        api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "space_track_sync_error",
            error.to_string(),
        )
    })?;
    let mut status = state.space_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    status.remembered_credentials = remembered_credentials(&state, &headers).is_some();
    Ok(Json(status))
}

fn remembered_credentials(state: &AppState, headers: &HeaderMap) -> Option<RememberedCredentials> {
    state
        .credential_cookie
        .open(headers.get("cookie").and_then(|value| value.to_str().ok()))
}

fn set_cookie_headers(cookie: String) -> Result<HeaderMap, (StatusCode, Json<ErrorResponse>)> {
    let value = HeaderValue::from_str(&cookie).map_err(|_| {
        api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "credential_cookie_error",
            "could not construct credential cookie",
        )
    })?;
    let mut headers = HeaderMap::new();
    headers.insert(SET_COOKIE, value);
    Ok(headers)
}
fn require_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let Some(token) = state.admin_token.as_ref() else {
        return Ok(());
    };
    let expected = format!("Bearer {token}");
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return Err(api_error(
            StatusCode::UNAUTHORIZED,
            "admin_token_required",
            "valid admin setup token required",
        ));
    }
    Ok(())
}

async fn stream_projection(
    ws: WebSocketUpgrade,
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ProjectionQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_socket(socket, state, game_id, query))
}
async fn stream_socket(
    mut socket: WebSocket,
    state: AppState,
    game_id: Uuid,
    query: ProjectionQuery,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let projection = {
            let mut games = state.games.write().await;
            let Some(game) = games.get_mut(&game_id) else {
                return;
            };
            let Ok(role) = authorized_role(game, &query) else {
                return;
            };
            game.simulation
                .projection_for(role.location_unit_id, role.side)
        };
        let Ok(payload) = serde_json::to_string(&projection) else {
            return;
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }
}

async fn run_simulation_loop(state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let mut games = state.games.write().await;
        for game in games
            .values_mut()
            .filter(|game| game.status == GameStatus::Running)
        {
            let ai_roles: Vec<Role> = game
                .roles
                .values()
                .filter(|role| role.ai_controlled)
                .cloned()
                .collect();
            for role in ai_roles {
                let Some(controlled) = role.command_units.first().copied() else {
                    continue;
                };
                let projection = game.simulation.projection_for(controlled, role.side);
                if let Some(intent) = choose_patrol_intent(role.id, controlled, &projection) {
                    let _ = submit_authority_action(
                        game,
                        role.id,
                        intent.kind.action_key().into(),
                        intent.target,
                        "AI patrol".into(),
                        Some(intent),
                    );
                }
            }
            process_vacant_authority_requests(game);
            game.simulation.step();
        }
    }
}

fn game_summary(game: &Game) -> GameSummary {
    GameSummary {
        id: game.id,
        title: game.title.clone(),
        status: game.status,
        host_player_id: game.host,
        player_roles_available: game
            .roles
            .values()
            .filter(|role| role.claimable && !role.ai_controlled && role.owner.is_none())
            .count(),
    }
}
fn role_summary(role: &Role) -> RoleSummary {
    RoleSummary {
        id: role.id,
        name: role.name.clone(),
        side: role.side,
        kind: role.kind,
        location_unit_id: role.location_unit_id,
        command_units: role.command_units.clone(),
        held: role.owner.is_some(),
        ai_controlled: role.ai_controlled,
        lease_generation: role.lease_generation,
    }
}
fn api_error(
    status: StatusCode,
    code: &'static str,
    error: impl Into<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            code,
            error: error.into(),
        }),
    )
}

#[cfg(test)]
mod authority_tests {
    use super::*;
    use sim_core::{OrderKind, OrderStatus, ACTION_SPACE_SUPPORT};

    fn game() -> Game {
        let scenario = global_crisis_scenario();
        let authority = scenario.authority.clone();
        let roles = authority
            .roles
            .iter()
            .map(|definition| {
                (
                    definition.id,
                    Role {
                        id: definition.id,
                        name: definition.name.clone(),
                        side: definition.side,
                        kind: definition.kind,
                        location_unit_id: definition.location_unit_id,
                        command_units: authority.controlled_units(definition.id),
                        claimable: definition.claimable,
                        owner: None,
                        ai_controlled: definition.ai_controlled,
                        lease_generation: 0,
                    },
                )
            })
            .collect();
        Game {
            id: Uuid::from_u128(900),
            title: "Test".into(),
            host: Uuid::from_u128(901),
            status: GameStatus::Running,
            simulation: scenario.spawn().unwrap(),
            roles,
            authority,
            authority_requests: BTreeMap::new(),
            authority_events: Vec::new(),
            unit_ids: scenario.units.iter().map(|unit| unit.id).collect(),
            space_catalog_checksum: String::new(),
        }
    }

    #[test]
    fn direct_order_is_wrapped_and_executed() {
        let mut game = game();
        let role = Uuid::from_u128(106);
        let target = Uuid::from_u128(5);
        let intent = PlayerIntent {
            intent_id: Uuid::from_u128(902),
            issuer_role: role,
            target,
            kind: OrderKind::Move {
                north_mps: 10.0,
                east_mps: 0.0,
            },
            requested_tick: 1,
        };
        assert!(matches!(
            submit_authority_action(
                &mut game,
                role,
                "move".into(),
                target,
                String::new(),
                Some(intent)
            )
            .unwrap(),
            SubmissionOutcome::Queued { .. }
        ));
        game.simulation.step();
        assert!(matches!(
            game.simulation.drain_order_results()[0].status,
            OrderStatus::Accepted
        ));
    }

    #[test]
    fn occupied_decider_waits_for_a_human() {
        let mut game = game();
        let decider = Uuid::from_u128(112);
        game.roles.get_mut(&decider).unwrap().owner = Some(Uuid::from_u128(903));
        let outcome = submit_authority_action(
            &mut game,
            Uuid::from_u128(106),
            ACTION_SPACE_SUPPORT.into(),
            Uuid::from_u128(47),
            "Need collection".into(),
            None,
        )
        .unwrap();
        let SubmissionOutcome::PendingAuthority { request_id } = outcome else {
            panic!("request expected")
        };
        for _ in 0..65 {
            game.simulation.step();
            process_vacant_authority_requests(&mut game);
        }
        assert!(
            matches!(game.authority_requests[&request_id].status, AuthorityRequestStatus::PendingHuman { role_id } if role_id == decider)
        );
    }

    #[test]
    fn vacant_decider_resolves_deterministically_after_delay() {
        let mut game = game();
        let policy = game
            .authority
            .policies
            .iter_mut()
            .find(|policy| policy.action == ACTION_SPACE_SUPPORT)
            .unwrap();
        policy.decision_steps[0].vacant_delay_ticks = 1;
        policy.decision_steps[0].approve_probability_bps = 10_000;
        let outcome = submit_authority_action(
            &mut game,
            Uuid::from_u128(106),
            ACTION_SPACE_SUPPORT.into(),
            Uuid::from_u128(47),
            "Need collection".into(),
            None,
        )
        .unwrap();
        let SubmissionOutcome::PendingAuthority { request_id } = outcome else {
            panic!("request expected")
        };
        game.simulation.step();
        process_vacant_authority_requests(&mut game);
        assert!(matches!(
            game.authority_requests[&request_id].status,
            AuthorityRequestStatus::ApprovedNoExecutor
        ));
        assert!(game.authority_requests[&request_id].decisions[0].automatic);
    }
}
