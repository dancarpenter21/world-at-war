mod credential_cookie;
mod space_catalog;

use std::{collections::BTreeMap, net::SocketAddr, sync::Arc, time::Duration};

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
use sim_core::{OrderResult, PlayerIntent, RoleProjection, Side, Simulation};
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
    command_scope: Vec<Uuid>,
    authority_echelon: u8,
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
    command_scope: Vec<Uuid>,
    authority_echelon: u8,
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
#[derive(Serialize)]
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
                role_count: scenario.roles.len(),
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
    let roles = scenario
        .roles
        .iter()
        .map(|template| {
            (
                template.id,
                Role {
                    id: template.id,
                    name: template.name.clone(),
                    side: template.side,
                    command_scope: template.command_scope.clone(),
                    authority_echelon: template.authority_echelon,
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
    if role.ai_controlled {
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
) -> ApiResult<Vec<OrderResult>> {
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
    if request.intent.issuer_role != role_id || !role.command_scope.contains(&request.intent.target)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "command_scope",
            "intent exceeds role command scope",
        ));
    }
    game.simulation.queue_intent(request.intent);
    Ok(Json(Vec::new()))
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
            .projection_for(role.command_scope[0], role.side),
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
                .projection_for(role.command_scope[0], role.side)
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
                let controlled = role.command_scope[0];
                let projection = game.simulation.projection_for(controlled, role.side);
                if let Some(intent) = choose_patrol_intent(role.id, controlled, &projection) {
                    game.simulation.queue_intent(intent);
                }
            }
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
            .filter(|role| !role.ai_controlled && role.owner.is_none())
            .count(),
    }
}
fn role_summary(role: &Role) -> RoleSummary {
    RoleSummary {
        id: role.id,
        name: role.name.clone(),
        side: role.side,
        command_scope: role.command_scope.clone(),
        authority_echelon: role.authority_echelon,
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
