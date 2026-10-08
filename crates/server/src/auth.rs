//! Browser guest credentials are independent of scenario time and role knowledge.
use super::*;
use axum::{
    extract::{FromRequest, FromRequestParts, Request},
    http::{request::Parts, Method},
    middleware::Next,
    response::Response,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::{sync::Mutex, time::Instant};

const COOKIE: &str = "world_at_war_session";
const SESSION_SECONDS: u64 = 30 * 24 * 60 * 60;
type Rejection = (StatusCode, Json<ErrorResponse>);

#[derive(Clone)]
pub(super) struct Session {
    pub player_id: Uuid,
    display_name: String,
    csrf: String,
    expires: Instant,
    expires_unix: u64,
    key: String,
}
#[derive(Serialize)]
struct SessionView {
    player_id: Uuid,
    display_name: String,
    csrf_token: String,
    expires_unix: u64,
}
impl Session {
    fn view(&self) -> SessionView {
        SessionView {
            player_id: self.player_id,
            display_name: self.display_name.clone(),
            csrf_token: self.csrf.clone(),
            expires_unix: self.expires_unix,
        }
    }
}
pub(super) struct Sessions {
    entries: Mutex<BTreeMap<String, Session>>,
    pub origins: Vec<HeaderValue>,
    secure: bool,
}
fn secret() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("operating system random source unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
impl Sessions {
    pub fn new() -> anyhow::Result<Self> {
        let origins = std::env::var("APP_ALLOWED_ORIGINS").ok().filter(|value| !value.trim().is_empty()).unwrap_or_else(|| "http://localhost:5173,http://127.0.0.1:5173,http://localhost:8080,http://127.0.0.1:8080,http://localhost:4173,http://127.0.0.1:4173".into())
            .split(',').map(str::trim).filter(|s| !s.is_empty()).map(HeaderValue::from_str).collect::<Result<Vec<_>, _>>()?;
        anyhow::ensure!(!origins.is_empty(), "APP_ALLOWED_ORIGINS must not be empty");
        Ok(Self {
            entries: Mutex::new(BTreeMap::new()),
            origins,
            secure: std::env::var("COOKIE_SECURE")
                .is_ok_and(|s| matches!(s.as_str(), "1" | "true" | "yes")),
        })
    }
    fn find(&self, headers: &HeaderMap) -> Option<Session> {
        let token = headers
            .get("cookie")?
            .to_str()
            .ok()?
            .split(';')
            .filter_map(|s| s.trim().split_once('='))
            .find_map(|(name, value)| (name == COOKIE).then_some(value))?;
        self.entries
            .lock()
            .unwrap()
            .get(&hash(token))
            .filter(|s| s.expires > Instant::now())
            .cloned()
    }
    pub fn valid(&self, session: &Session) -> bool {
        self.entries
            .lock()
            .unwrap()
            .get(&session.key)
            .is_some_and(|s| s.expires > Instant::now())
    }
    fn cookie(&self, token: &str, age: u64) -> HeaderValue {
        HeaderValue::from_str(&format!(
            "{COOKIE}={token}; Path=/v1; Max-Age={age}; HttpOnly; SameSite=Strict{}",
            if self.secure { "; Secure" } else { "" }
        ))
        .unwrap()
    }
    pub fn expired_players(&self) -> Vec<Uuid> {
        let mut entries = self.entries.lock().unwrap();
        let now = Instant::now();
        let players = entries
            .values()
            .filter(|s| s.expires <= now)
            .map(|s| s.player_id)
            .collect();
        entries.retain(|_, s| s.expires > now);
        players
    }
}
fn denied(code: &'static str, message: &'static str) -> Rejection {
    api_error(StatusCode::FORBIDDEN, code, message)
}
fn required() -> Rejection {
    api_error(
        StatusCode::UNAUTHORIZED,
        "session_required",
        "Guest session expired or unavailable",
    )
}

/// Protect the game boundary before any handler can inspect caller-supplied data.
pub(super) async fn guard(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let is_game = path == "/v1/games" || path.starts_with("/v1/games/");
    let is_auth = path.starts_with("/v1/auth/");
    let mutation = !matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS
    );
    let websocket = request
        .headers()
        .get("upgrade")
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    if is_game {
        let values: Vec<(String, String)> =
            match serde_urlencoded::from_str(request.uri().query().unwrap_or_default()) {
                Ok(values) => values,
                Err(_) => {
                    return api_error(StatusCode::BAD_REQUEST, "invalid_query", "Invalid query")
                        .into_response()
                }
            };
        if let Err(error) = reject_identity(values.iter().map(|(key, _)| key.as_str())) {
            return error.into_response();
        }
    }
    let origin = request.headers().get("origin");
    if (mutation || websocket) && origin.is_some_and(|o| !state.sessions.origins.contains(o)) {
        return denied("origin_not_allowed", "Request origin is not allowed").into_response();
    }
    // Guest bootstrap has no CSRF token yet, so requires an explicitly trusted origin.
    if path == "/v1/auth/guest"
        && mutation
        && !origin.is_some_and(|o| state.sessions.origins.contains(o))
    {
        return denied(
            "origin_required",
            "Guest creation requires an allowed Origin header",
        )
        .into_response();
    }
    if websocket && !origin.is_some_and(|o| state.sessions.origins.contains(o)) {
        return denied(
            "origin_required",
            "WebSocket requires an allowed Origin header",
        )
        .into_response();
    }
    let session = state.sessions.find(request.headers());
    if (is_game || (is_auth && path != "/v1/auth/guest")) && session.is_none() {
        return required().into_response();
    }
    if let Some(session) = session {
        if mutation
            && path != "/v1/auth/guest"
            && request
                .headers()
                .get("x-csrf-token")
                .and_then(|h| h.to_str().ok())
                != Some(session.csrf.as_str())
        {
            return denied("csrf_required", "Valid CSRF token required").into_response();
        }
        request.extensions_mut().insert(session);
    }
    let mut response = next.run(request).await;
    if is_game || is_auth {
        response.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        );
    }
    response
}
impl<S: Send + Sync> FromRequestParts<S> for Session {
    type Rejection = Rejection;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Session>()
            .cloned()
            .ok_or_else(required)
    }
}

// Keep trusted domain command DTOs separate from their wire identity. These
// extractors reject identity assertions and supply the authenticated principal.
pub(super) struct AuthJson<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for AuthJson<T> {
    type Rejection = Rejection;
    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let session = request
            .extensions()
            .get::<Session>()
            .cloned()
            .ok_or_else(required)?;
        let Json(mut value) =
            Json::<serde_json::Map<String, serde_json::Value>>::from_request(request, state)
                .await
                .map_err(|_| {
                    api_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_json",
                        "Expected a JSON object",
                    )
                })?;
        reject_identity(value.keys().map(String::as_str))?;
        value.insert("player_id".into(), serde_json::json!(session.player_id));
        value.insert(
            "host_player_id".into(),
            serde_json::json!(session.player_id),
        );
        serde_json::from_value(serde_json::Value::Object(value))
            .map(Self)
            .map_err(|error| {
                api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_request",
                    error.to_string(),
                )
            })
    }
}
pub(super) struct AuthQuery<T>(pub T);
impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for AuthQuery<T> {
    type Rejection = Rejection;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let session = parts.extensions.get::<Session>().ok_or_else(required)?;
        let mut values: Vec<(String, String)> =
            serde_urlencoded::from_str(parts.uri.query().unwrap_or_default()).map_err(|_| {
                api_error(StatusCode::BAD_REQUEST, "invalid_query", "Invalid query")
            })?;
        reject_identity(values.iter().map(|(key, _)| key.as_str()))?;
        values.push(("player_id".into(), session.player_id.to_string()));
        let encoded = serde_urlencoded::to_string(values).unwrap();
        serde_urlencoded::from_str(&encoded)
            .map(Self)
            .map_err(|error| api_error(StatusCode::BAD_REQUEST, "invalid_query", error.to_string()))
    }
}
fn reject_identity<'a>(keys: impl Iterator<Item = &'a str>) -> Result<(), Rejection> {
    if keys
        .into_iter()
        .any(|key| matches!(key, "player_id" | "host_player_id"))
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "identity_from_session",
            "Player identity must come from the guest session",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GuestRequest {
    #[serde(default)]
    display_name: Option<String>,
}
pub(super) async fn guest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<GuestRequest>,
) -> Result<Response, Rejection> {
    if let Some(session) = state.sessions.find(&headers) {
        return Ok(Json(session.view()).into_response());
    }
    let name = request.display_name.unwrap_or_else(|| "Commander".into());
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_display_name",
            "Display name must contain 1–80 characters",
        ));
    }
    let token = secret();
    let session = Session {
        player_id: Uuid::new_v4(),
        display_name: name.into(),
        csrf: secret(),
        expires: Instant::now() + Duration::from_secs(SESSION_SECONDS),
        expires_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + SESSION_SECONDS,
        key: hash(&token),
    };
    let mut response = Json(session.view()).into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, state.sessions.cookie(&token, SESSION_SECONDS));
    eprintln!("guest_created player={}", session.player_id);
    state
        .sessions
        .entries
        .lock()
        .unwrap()
        .insert(session.key.clone(), session);
    Ok(response)
}
pub(super) async fn current(session: Session) -> Response {
    Json(session.view()).into_response()
}
pub(super) async fn logout(State(state): State<AppState>, session: Session) -> Response {
    state.sessions.entries.lock().unwrap().remove(&session.key);
    let mut games = state.games.write().await;
    for game in games.values_mut() {
        leases::release_player(game, session.player_id);
    }
    eprintln!("guest_logged_out player={}", session.player_id);
    let mut response = Json(serde_json::json!({"logged_out": true})).into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, state.sessions.cookie("", 0));
    response
}

#[cfg(test)]
mod tests {
    include!("observer_auth_tests.rs");
    use super::*;
    use axum::body::{to_bytes, Body};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    struct Fixture {
        state: AppState,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.state.network_event_dir.as_ref());
        }
    }
    async fn fixture() -> Fixture {
        let directory = std::env::temp_dir().join(format!("world-at-war-auth-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let scenarios = [
            command_link_exercise_scenario(),
            regional_campaign_scenario(),
        ];
        Fixture {
            state: AppState {
                sessions: Arc::new(Sessions {
                    entries: Mutex::new(BTreeMap::new()),
                    origins: vec![HeaderValue::from_static("http://localhost:5173")],
                    secure: true,
                }),
                games: Arc::new(RwLock::new(BTreeMap::new())),
                scenarios: Arc::new(scenarios.into_iter().map(|s| (s.id.clone(), s)).collect()),
                airport_catalog: AirportCatalogService::load().await,
                space_catalog: SpaceCatalogService::load().await.unwrap(),
                space_assets: SpaceAssetService::load().await,
                admin_token: Arc::new(None),
                credential_cookie: CredentialCookie::ephemeral(),
                communications_catalog: Arc::new(
                    CommunicationsCatalog::load(
                        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                            .join("../../data/communications/catalog.yaml"),
                    )
                    .unwrap(),
                ),
                network_event_dir: Arc::new(directory),
            },
        }
    }
    async fn call(
        state: &AppState,
        method: &str,
        path: &str,
        cookie: &str,
        csrf: &str,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        let request = axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("origin", "http://localhost:5173")
            .header("cookie", cookie)
            .header("x-csrf-token", csrf)
            .header("content-type", "application/json")
            .body(if method == "GET" {
                Body::empty()
            } else {
                Body::from(body.to_string())
            })
            .unwrap();
        let response = router(state.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn new_guest(state: &AppState) -> (String, String, Uuid) {
        let (status, headers, view) =
            call(state, "POST", "/v1/auth/guest", "", "", json!({})).await;
        assert_eq!(status, StatusCode::OK);
        let header = headers[SET_COOKIE].to_str().unwrap();
        for attribute in ["HttpOnly", "SameSite=Strict", "Secure"] {
            assert!(header.contains(attribute));
        }
        let cookie = header.split(';').next().unwrap().to_owned();
        assert!(!cookie.contains(view["player_id"].as_str().unwrap()));
        (
            cookie,
            view["csrf_token"].as_str().unwrap().into(),
            serde_json::from_value(view["player_id"].clone()).unwrap(),
        )
    }
    async fn create_game(state: &AppState, cookie: &str, csrf: &str) -> (String, String) {
        let (status, _, created) = call(
            state,
            "POST",
            "/v1/games",
            cookie,
            csrf,
            json!({"scenario_id":"command-link-exercise.v1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{created}");
        let prefix = format!("/v1/games/{}", created["game"]["id"].as_str().unwrap());
        let (_, _, roles) = call(
            state,
            "GET",
            &format!("{prefix}/roles"),
            cookie,
            csrf,
            Value::Null,
        )
        .await;
        let role = roles
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["name"] == "Exercise Commander")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        (prefix, role)
    }
    #[tokio::test]
    async fn http_identity_csrf_and_host_checks_use_the_cookie() {
        let fixture = fixture().await;
        let state = &fixture.state;
        assert_eq!(
            call(state, "GET", "/v1/games", "", "", Value::Null).await.0,
            StatusCode::UNAUTHORIZED
        );
        let (cookie, csrf, player) = new_guest(state).await;
        let (_, _, same) = call(state, "POST", "/v1/auth/guest", &cookie, "", json!({})).await;
        assert_eq!(same["player_id"], player.to_string());
        assert_eq!(
            call(
                state,
                "POST",
                "/v1/games",
                &cookie,
                "wrong",
                json!({"scenario_id":"command-link-exercise.v1"})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                state,
                "POST",
                "/v1/games",
                &cookie,
                &csrf,
                json!({"scenario_id":"command-link-exercise.v1", "host_player_id": player})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        let (prefix, _) = create_game(state, &cookie, &csrf).await;
        let (other, other_csrf, _) = new_guest(state).await;
        assert_eq!(
            call(
                state,
                "POST",
                &format!("{prefix}/pause"),
                &other,
                &other_csrf,
                json!({})
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(
                state,
                "POST",
                &format!("{prefix}/pause"),
                &other,
                &other_csrf,
                json!({"player_id": player})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                state,
                "GET",
                &format!("{prefix}/roles?player_id={player}"),
                &other,
                "",
                Value::Null
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call(
                state,
                "GET",
                "/v1/games",
                "world_at_war_session=forged",
                "",
                Value::Null
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/v1/auth/guest")
            .header("origin", "https://untrusted.example")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        assert_eq!(
            router(state.clone())
                .oneshot(request)
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    #[tokio::test]
    async fn concurrent_claims_resume_and_logout_revoke_all_role_access() {
        let fixture = fixture().await;
        let state = &fixture.state;
        let (cookie, csrf, _) = new_guest(state).await;
        let (prefix, role) = create_game(state, &cookie, &csrf).await;
        let (other, other_csrf, _) = new_guest(state).await;
        let claim = format!("{prefix}/roles/{role}/claim");
        assert_eq!(
            call(state, "POST", &claim, &other, &other_csrf, json!({}))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        call(
            state,
            "POST",
            &format!("{prefix}/join"),
            &other,
            &other_csrf,
            json!({"display_name":"Other"}),
        )
        .await;
        let (first, second) = tokio::join!(
            call(state, "POST", &claim, &cookie, &csrf, json!({})),
            call(state, "POST", &claim, &other, &other_csrf, json!({}))
        );
        assert_eq!(
            [first.0, second.0]
                .iter()
                .filter(|s| **s == StatusCode::OK)
                .count(),
            1
        );
        let (cookie, csrf, held) = if first.0 == StatusCode::OK {
            (cookie, csrf, first.2)
        } else {
            (other, other_csrf, second.2)
        };
        let generation = held["lease_generation"].as_u64().unwrap();
        let role_prefix = format!("{prefix}/roles/{role}");
        let (_, _, resumed) = call(
            state,
            "POST",
            &format!("{role_prefix}/resume"),
            &cookie,
            &csrf,
            json!({}),
        )
        .await;
        assert_eq!(resumed["lease_generation"], generation);
        let picture = format!("{prefix}/state?role_id={role}&lease_generation={generation}");
        assert_eq!(
            call(state, "GET", &picture, &cookie, &csrf, Value::Null)
                .await
                .0,
            StatusCode::OK
        );
        let game_id: Uuid = prefix.rsplit('/').next().unwrap().parse().unwrap();
        {
            let mut games = state.games.write().await;
            let game = games.get_mut(&game_id).unwrap();
            game.status = GameStatus::Paused;
            game.roles.get_mut(&role.parse().unwrap()).unwrap().lease =
                Some(leases::Lease::reserved_for_test());
        }
        assert_eq!(
            call(state, "GET", &picture, &cookie, &csrf, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let (_, _, resumed) = call(
            state,
            "POST",
            &format!("{role_prefix}/resume"),
            &cookie,
            &csrf,
            json!({}),
        )
        .await;
        assert_eq!(resumed["lease_generation"], generation + 1);
        assert_eq!(
            call(state, "GET", &picture, &cookie, &csrf, Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let current_picture = format!(
            "{prefix}/state?role_id={role}&lease_generation={}",
            generation + 1
        );
        assert_eq!(
            call(state, "GET", &current_picture, &cookie, &csrf, Value::Null)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            call(state, "POST", "/v1/auth/logout", &cookie, &csrf, json!({}))
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            call(state, "GET", &current_picture, &cookie, &csrf, Value::Null)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        assert!(
            state.games.read().await[&game_id].roles[&role.parse().unwrap()]
                .owner
                .is_none()
        );
    }
    #[tokio::test]
    async fn session_expiry_cannot_be_extended_by_a_request() {
        let fixture = fixture().await;
        let state = &fixture.state;
        let (cookie, csrf, player) = new_guest(state).await;
        state
            .sessions
            .entries
            .lock()
            .unwrap()
            .values_mut()
            .for_each(|s| s.expires = Instant::now());
        assert_eq!(
            call(
                state,
                "GET",
                "/v1/auth/session",
                &cookie,
                &csrf,
                Value::Null
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(state.sessions.expired_players(), vec![player]);
    }
}
