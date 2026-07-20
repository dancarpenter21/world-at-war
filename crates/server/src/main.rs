mod airport_catalog;
mod credential_cookie;
mod space_assets;
mod space_catalog;

use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use airport_catalog::{AirportCatalogService, AirportCatalogStatus};
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
use sim_catalog::{
    airport::{
        evaluate_airport, Airport, AirportKind, MilitaryUse, RunwayCompatibilityAssessment,
        RunwayCompatibilityRequest,
    },
    space::{SatelliteAuthorityAssignment, SatelliteAuthorityKind, SourceReference},
};
use sim_core::{
    AuthorityDefinition, AuthorityPolicy, AuthorityRoleKind, AuthorizationRecord, AuthorizedIntent,
    PlayerIntent, RoleProjection, Side, Simulation,
};
use sim_scenario::{global_crisis_scenario, jammed_flight_scenario, Scenario};
use space_assets::{SpaceAssetDetail, SpaceAssetService, SpaceAssetsResponse};
use space_catalog::{SpaceCatalogService, SpaceCatalogSnapshot, SpaceCatalogStatus};
use tokio::sync::RwLock;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

const EXTERNAL_OPERATOR_DELAY_TICKS: u64 = 60;
const EXTERNAL_OPERATOR_APPROVAL_BPS: u16 = 5_000;

#[derive(Clone)]
struct AppState {
    games: Arc<RwLock<BTreeMap<Uuid, Game>>>,
    scenarios: Arc<BTreeMap<String, Scenario>>,
    airport_catalog: AirportCatalogService,
    space_catalog: SpaceCatalogService,
    space_assets: SpaceAssetService,
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
    space_catalog_checksum: Option<String>,
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
    space_catalog_enabled: bool,
}
#[derive(Deserialize)]
struct AirportListQuery {
    query: Option<String>,
    country: Option<String>,
    facility_use: Option<String>,
    minimum_runway_length_m: Option<f64>,
    west: Option<f64>,
    south: Option<f64>,
    east: Option<f64>,
    north: Option<f64>,
    horizon_latitude: Option<f64>,
    horizon_longitude: Option<f64>,
    horizon_radius_deg: Option<f64>,
    limit: Option<usize>,
    offset: Option<usize>,
}
#[derive(Serialize)]
struct AirportSummary {
    id: String,
    name: String,
    kind: AirportKind,
    country_code: String,
    region_code: Option<String>,
    municipality: Option<String>,
    military_use: MilitaryUse,
    latitude_deg: f64,
    longitude_deg: f64,
    runway_count: usize,
    longest_runway_m: Option<f64>,
}
#[derive(Serialize)]
struct AirportListResponse {
    checksum: String,
    total: usize,
    limit: usize,
    offset: usize,
    airports: Vec<AirportSummary>,
}
#[derive(Serialize)]
struct AirportCompatibilityResponse {
    catalog_checksum: String,
    airport_id: String,
    assessments: Vec<RunwayCompatibilityAssessment>,
}
#[derive(Deserialize)]
struct ForceSyncQuery {
    force: Option<bool>,
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
    PendingExternal {
        authority_id: String,
        resolves_at_tick: u64,
    },
    Approved,
    ApprovedNoExecutor,
    Denied {
        role_id: Uuid,
    },
    DeniedExternal {
        authority_id: String,
    },
    BlockedComms,
}
#[derive(Debug, Clone, Serialize)]
struct AuthorityRequest {
    id: Uuid,
    action: String,
    target_unit_id: Uuid,
    target: AuthorityTarget,
    requester_role_id: Uuid,
    policy: AuthorityPolicy,
    policy_version: u64,
    current_step: usize,
    created_tick: u64,
    summary: String,
    status: AuthorityRequestStatus,
    decisions: Vec<AuthorityDecisionRecord>,
    satellite_context: Option<FrozenSatelliteContext>,
    #[serde(skip)]
    intent: Option<PlayerIntent>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AuthorityTarget {
    Unit { unit_id: Uuid },
    Satellite { norad_catalog_id: u64 },
}
#[derive(Debug, Clone, Serialize)]
struct FrozenSatelliteContext {
    authority_assignment: SatelliteAuthorityAssignment,
    catalog_checksum: String,
    card_manifest_version: Option<String>,
    requester_role_id: Uuid,
    public_sources: Vec<SourceReference>,
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
struct CreateSatelliteRequest {
    player_id: Uuid,
    lease_generation: u64,
    action: String,
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
    let scenarios = [global_crisis_scenario(), jammed_flight_scenario()];
    for scenario in &scenarios {
        scenario.validate()?;
    }
    let admin_token = std::env::var("ADMIN_SETUP_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty());
    let state = AppState {
        games: Arc::new(RwLock::new(BTreeMap::new())),
        scenarios: Arc::new(
            scenarios
                .into_iter()
                .map(|scenario| (scenario.id.clone(), scenario))
                .collect(),
        ),
        airport_catalog: AirportCatalogService::load().await,
        space_catalog: SpaceCatalogService::load().await?,
        space_assets: SpaceAssetService::load().await,
        admin_token: Arc::new(admin_token),
        credential_cookie: CredentialCookie::load().await?,
    };
    tokio::spawn(run_simulation_loop(state.clone()));
    let airport_catalog = state.airport_catalog.clone();
    tokio::spawn(async move {
        airport_catalog.refresh_if_stale().await;
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/airport-catalog/status", get(airport_catalog_status))
        .route("/v1/airports", get(list_airports))
        .route("/v1/airports/{airport_id}", get(get_airport))
        .route(
            "/v1/airports/{airport_id}/compatibility",
            post(evaluate_airport_compatibility),
        )
        .route("/v1/admin/airport-catalog/sync", post(sync_airport_catalog))
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
        .route("/v1/games/{game_id}/space-assets", get(game_space_assets))
        .route(
            "/v1/games/{game_id}/space-assets/{norad_id}",
            get(game_space_asset),
        )
        .route(
            "/v1/games/{game_id}/roles/{role_id}/space-assets/{norad_id}/requests",
            post(create_satellite_request),
        )
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
    let checksum = if scenario.requires_space_catalog {
        let catalog = state.space_catalog.status().await;
        if !catalog.usable {
            return Err(api_error(
                StatusCode::CONFLICT,
                "space_catalog_unavailable",
                "connect Space-Track and synchronize a current catalog before creating this scenario",
            ));
        }
        Some(catalog.checksum.ok_or_else(|| {
            api_error(
                StatusCode::CONFLICT,
                "space_catalog_unavailable",
                "space catalog snapshot is missing",
            )
        })?)
    } else {
        None
    };
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

async fn create_satellite_request(
    Path((game_id, role_id, norad_id)): Path<(Uuid, Uuid, u64)>,
    State(state): State<AppState>,
    Json(request): Json<CreateSatelliteRequest>,
) -> ApiResult<SubmissionOutcome> {
    if request.summary.chars().count() > 500 {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "summary_too_long",
            "request summary is limited to 500 characters",
        ));
    }
    let checksum = {
        let games = state.games.read().await;
        let game = games
            .get(&game_id)
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
        validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
        require_game_catalog(game)?
    };
    let snapshot = state
        .space_catalog
        .snapshot(&checksum)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::GONE,
                "catalog_snapshot_missing",
                "the game's pinned space catalog is no longer available",
            )
        })?;
    let detail = state
        .space_assets
        .detail(&snapshot, norad_id)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "space_asset_not_found",
                "space asset not found",
            )
        })?;
    validate_satellite_request(&detail, &request.action)?;

    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    validate_role_lease(game, role_id, request.player_id, request.lease_generation)?;
    if game.space_catalog_checksum.as_deref() != Some(checksum.as_str()) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "catalog_pin_changed",
            "the game's pinned catalog changed while the request was prepared",
        ));
    }
    let policy_action = match request.action.as_str() {
        "request_satellite_service" => sim_core::ACTION_SPACE_SUPPORT,
        "coordinate_satellite_maneuver" => sim_core::ACTION_STRATEGIC_SATELLITE,
        _ => unreachable!("validated above"),
    };
    let policy = game
        .authority
        .policies
        .iter()
        .find(|policy| policy.action == policy_action)
        .cloned()
        .ok_or_else(|| {
            api_error(
                StatusCode::FORBIDDEN,
                "authority_not_defined",
                "no game authority policy covers this satellite request",
            )
        })?;
    if !policy.request_role_ids.contains(&role_id) && !policy.direct_role_ids.contains(&role_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "request_not_permitted",
            "this role may not make the satellite request",
        ));
    }
    let tick = game.simulation.tick();
    let request_id = Uuid::new_v4();
    let status = if detail.authority.kind == SatelliteAuthorityKind::MilitaryRole {
        let first_step = policy.decision_steps.first().ok_or_else(|| {
            api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "missing_decision_step",
                "authority policy has no decision step",
            )
        })?;
        status_for_decision_role(game, first_step, tick)
    } else {
        AuthorityRequestStatus::PendingExternal {
            authority_id: detail.authority.authority_id.clone(),
            resolves_at_tick: tick.saturating_add(EXTERNAL_OPERATOR_DELAY_TICKS),
        }
    };
    let frozen = FrozenSatelliteContext {
        authority_assignment: detail.authority.clone(),
        catalog_checksum: checksum,
        card_manifest_version: detail.manifest_version.clone(),
        requester_role_id: role_id,
        public_sources: detail.sources.clone(),
    };
    game.authority_requests.insert(
        request_id,
        AuthorityRequest {
            id: request_id,
            action: request.action,
            target_unit_id: Uuid::nil(),
            target: AuthorityTarget::Satellite {
                norad_catalog_id: norad_id,
            },
            requester_role_id: role_id,
            policy,
            policy_version: game.authority.version,
            current_step: 0,
            created_tick: tick,
            summary: request.summary,
            status,
            decisions: Vec::new(),
            satellite_context: Some(frozen.clone()),
            intent: None,
        },
    );
    game.authority_events.push(AuthorityEvent {
        tick,
        kind: "satellite_authority_requested".into(),
        detail: format!(
            "request {request_id}; frozen={}",
            serde_json::to_string(&frozen).unwrap_or_else(|_| "unavailable".into())
        ),
    });
    Ok(Json(SubmissionOutcome::PendingAuthority { request_id }))
}

fn validate_satellite_request(
    detail: &SpaceAssetDetail,
    action: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if !matches!(
        action,
        "request_satellite_service" | "coordinate_satellite_maneuver"
    ) {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_satellite_request",
            "supported actions are request_satellite_service and coordinate_satellite_maneuver",
        ));
    }
    if detail.record.object_type != "PAYLOAD" {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "payload_required",
            "requests may target payloads only",
        ));
    }
    if detail.record.nation != "US" {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "us_payload_required",
            "v1 requests may target U.S. payloads only",
        ));
    }
    if !detail.authority.commandable || detail.authority.kind == SatelliteAuthorityKind::Unresolved
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "authority_unresolved",
            "this payload has no reviewed commandable authority assignment",
        ));
    }
    if !detail
        .authority
        .allowed_request_types
        .iter()
        .any(|allowed| allowed == action)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "request_type_not_allowed",
            "the authority assignment does not allow this request type",
        ));
    }
    Ok(())
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
            target: AuthorityTarget::Unit { unit_id: target },
            requester_role_id: role_id,
            policy,
            policy_version: game.authority.version,
            current_step: 0,
            created_tick: tick,
            summary,
            status,
            decisions: Vec::new(),
            satellite_context: None,
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
        if let AuthorityRequestStatus::PendingExternal {
            authority_id,
            resolves_at_tick,
        } = request.status.clone()
        {
            if game.simulation.tick() >= resolves_at_tick {
                let sample =
                    ((request.id.as_u128() ^ request.policy_version as u128) % 10_000) as u16;
                request.decisions.push(AuthorityDecisionRecord {
                    role_id: Uuid::nil(),
                    approved: sample < EXTERNAL_OPERATOR_APPROVAL_BPS,
                    automatic: true,
                    tick: game.simulation.tick(),
                });
                request.status = if sample < EXTERNAL_OPERATOR_APPROVAL_BPS {
                    AuthorityRequestStatus::ApprovedNoExecutor
                } else {
                    AuthorityRequestStatus::DeniedExternal { authority_id }
                };
                game.authority_events.push(AuthorityEvent {
                    tick: game.simulation.tick(),
                    kind: "external_operator_decision".into(),
                    detail: format!(
                        "request {id} resolved by deterministic external-operator actor"
                    ),
                });
            }
            game.authority_requests.insert(id, request);
            continue;
        }
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
    let checksum = require_game_catalog(game)?;
    let snapshot = state
        .space_catalog
        .snapshot(&checksum)
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

async fn game_space_assets(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    Query(query): Query<ProjectionQuery>,
) -> Result<(HeaderMap, Json<SpaceAssetsResponse>), (StatusCode, Json<ErrorResponse>)> {
    let checksum = {
        let games = state.games.read().await;
        let game = games
            .get(&game_id)
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
        authorized_role(game, &query)?;
        require_game_catalog(game)?
    };
    let snapshot = state
        .space_catalog
        .snapshot(&checksum)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::GONE,
                "catalog_snapshot_missing",
                "the game's pinned space catalog is no longer available",
            )
        })?;
    let response = state.space_assets.list(&snapshot);
    let mut headers = HeaderMap::new();
    let tag = format!(
        "\"space-assets-{}-{}\"",
        checksum,
        response.manifest_version.as_deref().unwrap_or("baseline")
    );
    if let Ok(value) = HeaderValue::from_str(&tag) {
        headers.insert(axum::http::header::ETAG, value);
    }
    Ok((headers, Json(response)))
}

async fn game_space_asset(
    Path((game_id, norad_id)): Path<(Uuid, u64)>,
    State(state): State<AppState>,
    Query(query): Query<ProjectionQuery>,
) -> Result<(HeaderMap, Json<SpaceAssetDetail>), (StatusCode, Json<ErrorResponse>)> {
    let checksum = {
        let games = state.games.read().await;
        let game = games
            .get(&game_id)
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
        authorized_role(game, &query)?;
        require_game_catalog(game)?
    };
    let snapshot = state
        .space_catalog
        .snapshot(&checksum)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::GONE,
                "catalog_snapshot_missing",
                "the game's pinned space catalog is no longer available",
            )
        })?;
    let response = state
        .space_assets
        .detail(&snapshot, norad_id)
        .await
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "space_asset_not_found",
                "space asset not found",
            )
        })?;
    let mut headers = HeaderMap::new();
    let tag = format!(
        "\"space-card-{}-{}-{}\"",
        checksum,
        response.manifest_version.as_deref().unwrap_or("baseline"),
        norad_id
    );
    if let Ok(value) = HeaderValue::from_str(&tag) {
        headers.insert(axum::http::header::ETAG, value);
    }
    Ok((headers, Json(response)))
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

async fn airport_catalog_status(State(state): State<AppState>) -> Json<AirportCatalogStatus> {
    let mut status = state.airport_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    Json(status)
}

async fn list_airports(
    State(state): State<AppState>,
    Query(query): Query<AirportListQuery>,
) -> ApiResult<AirportListResponse> {
    if query
        .minimum_runway_length_m
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_runway_length",
            "minimum runway length must be non-negative",
        ));
    }
    let bounds = match (query.west, query.south, query.east, query.north) {
        (None, None, None, None) => None,
        (Some(west), Some(south), Some(east), Some(north))
            if [west, south, east, north]
                .iter()
                .all(|value| value.is_finite())
                && (-180.0..=180.0).contains(&west)
                && (-180.0..=180.0).contains(&east)
                && (-90.0..=90.0).contains(&south)
                && (-90.0..=90.0).contains(&north)
                && south <= north =>
        {
            Some((west, south, east, north))
        }
        _ => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "invalid_bounds",
                "west, south, east, and north must all be supplied as valid degree bounds",
            ));
        }
    };
    let horizon = match (
        query.horizon_latitude,
        query.horizon_longitude,
        query.horizon_radius_deg,
    ) {
        (None, None, None) => None,
        (Some(latitude), Some(longitude), Some(radius))
            if [latitude, longitude, radius]
                .iter()
                .all(|value| value.is_finite())
                && (-90.0..=90.0).contains(&latitude)
                && (-180.0..=180.0).contains(&longitude)
                && (0.0..=180.0).contains(&radius) =>
        {
            Some((latitude, longitude, radius))
        }
        _ => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "invalid_horizon",
                "horizon latitude, longitude, and radius must all be supplied as valid degree values",
            ));
        }
    };
    let snapshot = state.airport_catalog.snapshot().await.ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "airport_catalog_unavailable",
            "airport catalog is not ready",
        )
    })?;
    let search = query
        .query
        .as_deref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let country = query
        .country
        .as_deref()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty());
    let facility_use = query
        .facility_use
        .as_deref()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let mut matching: Vec<&Airport> = snapshot
        .airports
        .iter()
        .filter(|airport| {
            if bounds.is_some_and(|(west, south, east, north)| {
                airport.latitude_deg < south
                    || airport.latitude_deg > north
                    || !longitude_in_bounds(airport.longitude_deg, west, east)
            }) {
                return false;
            }
            if horizon.is_some_and(|(latitude, longitude, radius)| {
                angular_distance_degrees(
                    latitude,
                    longitude,
                    airport.latitude_deg,
                    airport.longitude_deg,
                ) > radius
            }) {
                return false;
            }
            if country
                .as_ref()
                .is_some_and(|country| airport.country_code.to_ascii_uppercase() != *country)
            {
                return false;
            }
            if query.minimum_runway_length_m.is_some_and(|minimum| {
                !airport
                    .runways
                    .iter()
                    .any(|runway| runway.length_m.is_some_and(|length| length >= minimum))
            }) {
                return false;
            }
            if facility_use
                .as_ref()
                .is_some_and(|filter| !airport_matches_use(airport, filter))
            {
                return false;
            }
            search.as_ref().is_none_or(|search| {
                airport.name.to_lowercase().contains(search)
                    || airport
                        .municipality
                        .as_ref()
                        .is_some_and(|value| value.to_lowercase().contains(search))
                    || airport
                        .identifiers
                        .values()
                        .any(|value| value.to_lowercase().contains(search))
            })
        })
        .collect();
    if bounds.is_some() {
        matching.sort_by(|left, right| {
            airport_map_priority(right)
                .cmp(&airport_map_priority(left))
                .then_with(|| longest_runway(right).total_cmp(&longest_runway(left)))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
    } else {
        matching.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    let total = matching.len();
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let offset = query.offset.unwrap_or(0);
    let airports = matching
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(airport_summary)
        .collect();
    Ok(Json(AirportListResponse {
        checksum: snapshot.checksum.clone(),
        total,
        limit,
        offset,
        airports,
    }))
}

async fn get_airport(
    State(state): State<AppState>,
    Path(airport_id): Path<String>,
) -> ApiResult<Airport> {
    let snapshot = state.airport_catalog.snapshot().await.ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "airport_catalog_unavailable",
            "airport catalog is not ready",
        )
    })?;
    snapshot
        .airports
        .iter()
        .find(|airport| airport.id == airport_id)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "airport_not_found",
                "airport not found",
            )
        })
}

async fn evaluate_airport_compatibility(
    State(state): State<AppState>,
    Path(airport_id): Path<String>,
    Json(request): Json<RunwayCompatibilityRequest>,
) -> ApiResult<AirportCompatibilityResponse> {
    let snapshot = state.airport_catalog.snapshot().await.ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "airport_catalog_unavailable",
            "airport catalog is not ready",
        )
    })?;
    let airport = snapshot
        .airports
        .iter()
        .find(|airport| airport.id == airport_id)
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "airport_not_found",
                "airport not found",
            )
        })?;
    let assessments = evaluate_airport(airport, &request).map_err(|error| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid_aircraft_requirements",
            error.to_string(),
        )
    })?;
    Ok(Json(AirportCompatibilityResponse {
        catalog_checksum: snapshot.checksum.clone(),
        airport_id,
        assessments,
    }))
}

async fn sync_airport_catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ForceSyncQuery>,
) -> ApiResult<AirportCatalogStatus> {
    require_admin(&state, &headers)?;
    state
        .airport_catalog
        .sync(query.force.unwrap_or(false))
        .await
        .map_err(|error| {
            let status = if error.to_string().contains("already running") {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_GATEWAY
            };
            api_error(status, "airport_catalog_sync_error", error.to_string())
        })?;
    let mut status = state.airport_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    Ok(Json(status))
}

fn airport_summary(airport: &Airport) -> AirportSummary {
    AirportSummary {
        id: airport.id.clone(),
        name: airport.name.clone(),
        kind: airport.kind,
        country_code: airport.country_code.clone(),
        region_code: airport.region_code.clone(),
        municipality: airport.municipality.clone(),
        military_use: airport.military_use,
        latitude_deg: airport.latitude_deg,
        longitude_deg: airport.longitude_deg,
        runway_count: airport.runways.len(),
        longest_runway_m: airport
            .runways
            .iter()
            .filter_map(|runway| runway.length_m)
            .reduce(f64::max),
    }
}

fn longest_runway(airport: &Airport) -> f64 {
    airport
        .runways
        .iter()
        .filter_map(|runway| runway.length_m)
        .reduce(f64::max)
        .unwrap_or(0.0)
}

fn airport_map_priority(airport: &Airport) -> u8 {
    match airport.kind {
        AirportKind::LargeAirport => 4,
        AirportKind::MediumAirport => 3,
        AirportKind::SmallAirport => 2,
        AirportKind::Unknown => 1,
        AirportKind::Heliport
        | AirportKind::SeaplaneBase
        | AirportKind::Balloonport
        | AirportKind::ClosedAirport => 0,
    }
}

fn longitude_in_bounds(longitude: f64, west: f64, east: f64) -> bool {
    if west <= east {
        (west..=east).contains(&longitude)
    } else {
        longitude >= west || longitude <= east
    }
}

fn angular_distance_degrees(
    left_latitude: f64,
    left_longitude: f64,
    right_latitude: f64,
    right_longitude: f64,
) -> f64 {
    let left_latitude = left_latitude.to_radians();
    let right_latitude = right_latitude.to_radians();
    let latitude_delta = right_latitude - left_latitude;
    let longitude_delta = (right_longitude - left_longitude).to_radians();
    let haversine = (latitude_delta / 2.0).sin().powi(2)
        + left_latitude.cos() * right_latitude.cos() * (longitude_delta / 2.0).sin().powi(2);
    2.0 * haversine.clamp(0.0, 1.0).sqrt().asin().to_degrees()
}

#[cfg(test)]
mod airport_query_tests {
    use super::{angular_distance_degrees, longitude_in_bounds};

    #[test]
    fn horizon_distance_and_antimeridian_bounds_are_spherical() {
        assert!((angular_distance_degrees(0.0, 179.0, 0.0, -179.0) - 2.0).abs() < 1e-9);
        assert!((angular_distance_degrees(0.0, 0.0, 0.0, 180.0) - 180.0).abs() < 1e-9);
        assert!(longitude_in_bounds(-179.0, 170.0, -170.0));
        assert!(!longitude_in_bounds(0.0, 170.0, -170.0));
    }
}

fn airport_matches_use(airport: &Airport, filter: &str) -> bool {
    match filter {
        "military" => airport.military_use == MilitaryUse::Military,
        "joint" | "joint_use" => airport.military_use == MilitaryUse::Joint,
        "civilian" => airport.military_use == MilitaryUse::Civilian,
        "unknown" => airport.military_use == MilitaryUse::Unknown,
        other => airport
            .facility_use
            .as_ref()
            .is_some_and(|value| value.eq_ignore_ascii_case(other)),
    }
}

async fn space_catalog_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<SpaceCatalogStatus> {
    let mut status = state.space_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    let remembered = remembered_credentials(&state, &headers);
    status.remembered_credentials = remembered.is_some();
    status.remembered_username = remembered.map(|credentials| credentials.username);
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
    status.remembered_username = request.remember.then(|| remembered.username.clone());
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
    let username = credentials.username;
    let mut status = state
        .space_catalog
        .restore_credentials(username.clone(), credentials.password)
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
    status.remembered_username = Some(username);
    Ok(Json(status))
}

async fn forget_space_track(State(state): State<AppState>) -> CookieApiResult<SpaceCatalogStatus> {
    state.space_catalog.clear_credentials().await;
    let mut status = state.space_catalog.status().await;
    status.setup_auth_required = state.admin_token.is_some();
    status.remembered_credentials = false;
    status.remembered_username = None;
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
    let remembered = remembered_credentials(&state, &headers);
    status.remembered_credentials = remembered.is_some();
    status.remembered_username = remembered.map(|credentials| credentials.username);
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
        space_catalog_enabled: game.space_catalog_checksum.is_some(),
    }
}

fn require_game_catalog(game: &Game) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    game.space_catalog_checksum.clone().ok_or_else(|| {
        api_error(
            StatusCode::CONFLICT,
            "space_catalog_not_enabled",
            "the selected scenario does not enable an orbital catalog",
        )
    })
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
    use sim_catalog::space::{
        AuthorityConfidence, SatelliteAuthorityAssignment, SatelliteAuthorityKind,
        SpaceAssetIndexEntry,
    };
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
            space_catalog_checksum: Some(String::new()),
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

    fn satellite_detail(
        object_type: &str,
        nation: &str,
        kind: SatelliteAuthorityKind,
        commandable: bool,
    ) -> SpaceAssetDetail {
        let authority = SatelliteAuthorityAssignment {
            authority_id: "test.authority".into(),
            display_name: "Test authority".into(),
            organization: "Test organization".into(),
            kind,
            game_role_name: None,
            public_source_ids: vec!["official".into()],
            confidence: AuthorityConfidence::Official,
            allowed_request_types: vec![
                "request_satellite_service".into(),
                "coordinate_satellite_maneuver".into(),
            ],
            commandable,
        };
        let record = SpaceAssetIndexEntry {
            norad_catalog_id: 5,
            cospar_id: Some("1958-002B".into()),
            canonical_name: "Test payload".into(),
            aliases: Vec::new(),
            nation: nation.into(),
            object_type: object_type.into(),
            orbital_regime: "leo".into(),
            operational_status: "Unknown".into(),
            operator: "Test".into(),
            mission_category: "Test".into(),
            public_description: Some("Public description".into()),
            sensors: vec!["Public sensor".into()],
            public_source_ids: vec!["official".into()],
            launch_year: Some(1958),
            radar_size_class: "MEDIUM".into(),
            inclination_deg: Some(34.0),
            authority: authority.clone(),
            has_enriched_card: true,
        };
        SpaceAssetDetail {
            catalog_checksum: "a".repeat(64),
            manifest_version: Some("1.0.0".into()),
            enrichment_available: true,
            record,
            raw_orbital_fields: serde_json::json!({}),
            markdown: String::new(),
            sources: Vec::new(),
            confidence: authority.confidence.clone(),
            authority,
        }
    }

    #[test]
    fn satellite_request_eligibility_rejects_debris_foreign_and_unresolved_targets() {
        assert!(validate_satellite_request(
            &satellite_detail("DEBRIS", "US", SatelliteAuthorityKind::MilitaryRole, true),
            "request_satellite_service"
        )
        .is_err());
        assert!(validate_satellite_request(
            &satellite_detail("PAYLOAD", "FR", SatelliteAuthorityKind::CivilOperator, true),
            "request_satellite_service"
        )
        .is_err());
        assert!(validate_satellite_request(
            &satellite_detail("PAYLOAD", "US", SatelliteAuthorityKind::Unresolved, false),
            "request_satellite_service"
        )
        .is_err());
        assert!(validate_satellite_request(
            &satellite_detail(
                "PAYLOAD",
                "US",
                SatelliteAuthorityKind::CommercialOperator,
                true
            ),
            "request_satellite_service"
        )
        .is_ok());
    }

    #[test]
    fn external_operator_resolves_without_an_executor_or_orbit_change() {
        let mut game = game();
        let policy = game
            .authority
            .policies
            .iter()
            .find(|policy| policy.action == ACTION_SPACE_SUPPORT)
            .unwrap()
            .clone();
        let request_id = Uuid::from_u128(9_999);
        game.authority_requests.insert(
            request_id,
            AuthorityRequest {
                id: request_id,
                action: "request_satellite_service".into(),
                target_unit_id: Uuid::nil(),
                target: AuthorityTarget::Satellite {
                    norad_catalog_id: 5,
                },
                requester_role_id: Uuid::from_u128(106),
                policy,
                policy_version: game.authority.version,
                current_step: 0,
                created_tick: 0,
                summary: "test".into(),
                status: AuthorityRequestStatus::PendingExternal {
                    authority_id: "commercial.test".into(),
                    resolves_at_tick: 1,
                },
                decisions: Vec::new(),
                satellite_context: None,
                intent: None,
            },
        );
        game.simulation.step();
        process_vacant_authority_requests(&mut game);
        let request = &game.authority_requests[&request_id];
        assert!(matches!(
            request.status,
            AuthorityRequestStatus::ApprovedNoExecutor
                | AuthorityRequestStatus::DeniedExternal { .. }
        ));
        assert!(request.decisions[0].automatic);
        assert!(request.intent.is_none());
    }
}
