//! Wall-clock seat ownership; never driven by simulation ticks.
use super::*;
use std::time::Instant;

const ACTIVE_SECONDS: u64 = 90;
const RESERVED_SECONDS: u64 = 300;
#[derive(Debug, Clone)]
pub(super) struct Lease {
    active_until: Instant,
    reserved_until: Instant,
}
impl Lease {
    #[cfg(test)]
    pub(super) fn reserved_for_test() -> Self {
        let active_until = Instant::now();
        Self {
            active_until,
            reserved_until: active_until + Duration::from_secs(RESERVED_SECONDS),
        }
    }
    pub fn new(now: Instant) -> Self {
        let active_until = now + Duration::from_secs(ACTIVE_SECONDS);
        Self {
            active_until,
            reserved_until: active_until + Duration::from_secs(RESERVED_SECONDS),
        }
    }
    pub fn active(&self, now: Instant) -> bool {
        now < self.active_until
    }
    pub fn expired(&self, now: Instant) -> bool {
        now >= self.reserved_until
    }
}
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum LeaseState {
    Available,
    Active,
    Reserved,
}
pub(super) fn active(role: &Role) -> bool {
    role.owner.is_some()
        && role
            .lease
            .as_ref()
            .is_some_and(|lease| lease.active(Instant::now()))
}
pub(super) fn summary(role: &Role, now: Instant) -> (LeaseState, u64) {
    match (&role.owner, &role.lease) {
        (Some(_), Some(lease)) if !lease.expired(now) => {
            if lease.active(now) {
                (
                    LeaseState::Active,
                    lease.active_until.saturating_duration_since(now).as_secs(),
                )
            } else {
                (
                    LeaseState::Reserved,
                    lease
                        .reserved_until
                        .saturating_duration_since(now)
                        .as_secs(),
                )
            }
        }
        _ => (LeaseState::Available, 0),
    }
}
pub(super) fn release(role: &mut Role) {
    if role.owner.take().is_some() {
        role.lease_generation += 1;
        role.lease = None;
        eprintln!(
            "role_released role={} generation={}",
            role.id, role.lease_generation
        );
    }
}
pub(super) fn expire(game: &mut Game, now: Instant) {
    for role in game.roles.values_mut() {
        if role.lease.as_ref().is_some_and(|lease| lease.expired(now)) {
            release(role);
        }
    }
}
pub(super) fn release_player(game: &mut Game, player: Uuid) {
    for role in game.roles.values_mut().filter(|r| r.owner == Some(player)) {
        release(role);
    }
}
fn acquire(
    role: &mut Role,
    player: Uuid,
    resume: bool,
    now: Instant,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if role.lease.as_ref().is_some_and(|l| l.expired(now)) {
        release(role);
    }
    if role.ai_controlled || !role.claimable {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "ai_role",
            "This role is not claimable",
        ));
    }
    if resume && role.owner != Some(player) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "role_not_held",
            "Previous role is no longer held by this guest",
        ));
    }
    if role.owner.is_some() && role.owner != Some(player) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "role_held",
            "Role is held or reserved",
        ));
    }
    if role.owner == Some(player) && role.lease.as_ref().is_some_and(|l| l.active(now)) {
        role.lease = Some(Lease::new(now));
        return Ok(());
    }
    role.owner = Some(player);
    role.lease_generation += 1;
    role.lease = Some(Lease::new(now));
    eprintln!(
        "role_acquired role={} player={} generation={}",
        role.id, player, role.lease_generation
    );
    Ok(())
}
pub(super) async fn claim(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_request): AuthJson<serde_json::Value>,
) -> ApiResult<RoleSummary> {
    acquire_role(state, game_id, role_id, session.player_id, false).await
}
pub(super) async fn resume(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_request): AuthJson<serde_json::Value>,
) -> ApiResult<RoleSummary> {
    acquire_role(state, game_id, role_id, session.player_id, true).await
}
async fn acquire_role(
    state: AppState,
    game_id: Uuid,
    role_id: Uuid,
    player: Uuid,
    resume: bool,
) -> ApiResult<RoleSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    if !game.members.contains(&player) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "join_required",
            "Join the game before claiming a role",
        ));
    }
    let role = game
        .roles
        .get_mut(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "Role not found"))?;
    acquire(role, player, resume, Instant::now())?;
    Ok(Json(role_summary(role, Some(player))))
}
#[derive(Deserialize)]
pub(super) struct LeaseRequest {
    lease_generation: u64,
}
pub(super) async fn renew(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(request): AuthJson<LeaseRequest>,
) -> ApiResult<RoleSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    validate_role_lease(game, role_id, session.player_id, request.lease_generation)?;
    let role = game.roles.get_mut(&role_id).unwrap();
    role.lease = Some(Lease::new(Instant::now()));
    Ok(Json(role_summary(role, Some(session.player_id))))
}
pub(super) async fn release_role(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(request): AuthJson<LeaseRequest>,
) -> ApiResult<RoleSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    expire(game, Instant::now());
    let role = game
        .roles
        .get_mut(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "Role not found"))?;
    if role.owner != Some(session.player_id) || role.lease_generation != request.lease_generation {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "invalid_role_lease",
            "Role lease no longer valid",
        ));
    }
    release(role);
    Ok(Json(role_summary(role, Some(session.player_id))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_boundaries_shared_claims_and_reserved_resume() {
        let mut game = authority_tests::game();
        let role = game.roles.get_mut(&Uuid::from_u128(106)).unwrap();
        let now = Instant::now();
        let player = Uuid::new_v4();
        acquire(role, player, false, now).unwrap();
        let generation = role.lease_generation;
        acquire(role, player, true, now + Duration::from_secs(1)).unwrap();
        assert_eq!(generation, role.lease_generation);
        assert_eq!(
            summary(role, now + Duration::from_secs(ACTIVE_SECONDS)).0,
            LeaseState::Active
        );
        let reserved = now + Duration::from_secs(ACTIVE_SECONDS + 1);
        assert_eq!(summary(role, reserved).0, LeaseState::Reserved);
        assert!(acquire(role, Uuid::new_v4(), false, reserved).is_err());
        acquire(role, player, true, reserved).unwrap();
        assert_eq!(role.lease_generation, generation + 1);
        let expired = reserved + Duration::from_secs(ACTIVE_SECONDS + RESERVED_SECONDS);
        assert!(acquire(role, player, true, expired).is_err());
        assert_eq!(summary(role, expired).0, LeaseState::Available);
        acquire(role, Uuid::new_v4(), false, expired).unwrap();
    }
    #[test]
    fn expiry_and_release_invalidate_generation_even_while_paused() {
        let mut game = authority_tests::game();
        game.status = GameStatus::Paused;
        let player = Uuid::new_v4();
        let id = Uuid::from_u128(106);
        let now = Instant::now();
        acquire(game.roles.get_mut(&id).unwrap(), player, false, now).unwrap();
        let generation = game.roles[&id].lease_generation;
        expire(
            &mut game,
            now + Duration::from_secs(ACTIVE_SECONDS + RESERVED_SECONDS),
        );
        assert!(validate_role_lease(&game, id, player, generation).is_err());
        assert!(game.roles[&id].owner.is_none());
        acquire(game.roles.get_mut(&id).unwrap(), player, false, now).unwrap();
        release_player(&mut game, player);
        assert!(game.roles[&id].owner.is_none());
    }
}
