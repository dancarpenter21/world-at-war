//! Wall-clock seat ownership; never driven by simulation ticks.
use super::*;
use std::time::Instant;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Occupancy {
    pub owner: Option<Uuid>,
    pub lease_generation: u64,
    pub lease: Option<Lease>,
}

const ACTIVE_SECONDS: u64 = 90;
const RESERVED_SECONDS: u64 = 300;
#[derive(Debug, Clone)]
pub(super) struct Lease {
    active_until: Instant,
    reserved_until: Instant,
    active_until_unix_ms: u64,
    reserved_until_unix_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredLease {
    active_until_unix_ms: u64,
    reserved_until_unix_ms: u64,
}
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
impl Serialize for Lease {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        StoredLease {
            active_until_unix_ms: self.active_until_unix_ms,
            reserved_until_unix_ms: self.reserved_until_unix_ms,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Lease {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = StoredLease::deserialize(deserializer)?;
        if saved.active_until_unix_ms > saved.reserved_until_unix_ms {
            return Err(serde::de::Error::custom("invalid lease deadlines"));
        }
        let now = Instant::now();
        let unix = unix_millis();
        let deadline = |at: u64| {
            now.checked_add(Duration::from_millis(at.saturating_sub(unix)))
                .ok_or_else(|| serde::de::Error::custom("lease deadline overflow"))
        };
        Ok(Self {
            active_until: deadline(saved.active_until_unix_ms)?,
            reserved_until: deadline(saved.reserved_until_unix_ms)?,
            active_until_unix_ms: saved.active_until_unix_ms,
            reserved_until_unix_ms: saved.reserved_until_unix_ms,
        })
    }
}
impl Lease {
    #[cfg(test)]
    pub(super) fn reserved_for_test() -> Self {
        let active_until = Instant::now();
        Self {
            active_until,
            reserved_until: active_until + Duration::from_secs(RESERVED_SECONDS),
            active_until_unix_ms: unix_millis(),
            reserved_until_unix_ms: unix_millis() + RESERVED_SECONDS * 1000,
        }
    }
    pub fn new(now: Instant) -> Self {
        let active_until = now + Duration::from_secs(ACTIVE_SECONDS);
        Self {
            active_until,
            reserved_until: active_until + Duration::from_secs(RESERVED_SECONDS),
            active_until_unix_ms: unix_millis() + ACTIVE_SECONDS * 1000,
            reserved_until_unix_ms: unix_millis() + (ACTIVE_SECONDS + RESERVED_SECONDS) * 1000,
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
pub(super) fn active(role: &Occupancy) -> bool {
    role.owner.is_some()
        && role
            .lease
            .as_ref()
            .is_some_and(|lease| lease.active(Instant::now()))
}
pub(super) fn summary(role: &Occupancy, now: Instant) -> (LeaseState, u64) {
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
pub(super) fn release(id: Uuid, role: &mut Occupancy) {
    if role.owner.take().is_some() {
        role.lease_generation += 1;
        role.lease = None;
        eprintln!(
            "role_released role={id} generation={}",
            role.lease_generation
        );
    }
}
pub(super) fn expire(game: &mut Game, now: Instant) {
    for (id, role) in game
        .roles
        .values_mut()
        .map(|r| (r.id, &mut r.occupancy))
        .chain(
            game.observers
                .values_mut()
                .map(|s| (s.definition.id, &mut s.occupancy)),
        )
    {
        if role.lease.as_ref().is_some_and(|lease| lease.expired(now)) {
            release(id, role);
        }
    }
}
pub(super) fn release_player(game: &mut Game, player: Uuid) {
    for (id, role) in game
        .roles
        .values_mut()
        .map(|r| (r.id, &mut r.occupancy))
        .chain(
            game.observers
                .values_mut()
                .map(|s| (s.definition.id, &mut s.occupancy)),
        )
        .filter(|(_, r)| r.owner == Some(player))
    {
        release(id, role);
    }
}
fn acquire(
    role: &mut Role,
    player: Uuid,
    resume: bool,
    now: Instant,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if role.ai_controlled || !role.claimable {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "ai_role",
            "This role is not claimable",
        ));
    }
    acquire_occupancy(role.id, &mut role.occupancy, player, resume, now)
}
pub(super) fn acquire_occupancy(
    id: Uuid,
    role: &mut Occupancy,
    player: Uuid,
    resume: bool,
    now: Instant,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if role.lease.as_ref().is_some_and(|l| l.expired(now)) {
        release(id, role);
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
        "role_acquired role={id} player={player} generation={}",
        role.lease_generation
    );
    Ok(())
}
pub(super) async fn claim(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_request): AuthJson<serde_json::Value>,
) -> ApiResult<observers::SeatSummary> {
    acquire_role(state, game_id, role_id, session.player_id, false).await
}
pub(super) async fn resume(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_request): AuthJson<serde_json::Value>,
) -> ApiResult<observers::SeatSummary> {
    acquire_role(state, game_id, role_id, session.player_id, true).await
}
async fn acquire_role(
    state: AppState,
    game_id: Uuid,
    role_id: Uuid,
    player: Uuid,
    resume: bool,
) -> ApiResult<observers::SeatSummary> {
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
    if game.observers.contains_key(&role_id) {
        return observers::acquire(game, role_id, player, resume);
    }
    if let Some(role) = game.roles.get(&role_id) {
        teams::validate_side(game, player, role.side)?;
        if teams::team(game, player).is_none()
            && game
                .roles
                .values()
                .any(|r| r.owner == Some(player) && r.side != role.side)
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "team_transfer_required",
                "Request an approved team transfer first",
            ));
        }
    }
    let role = game
        .roles
        .get_mut(&role_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "role_not_found", "Role not found"))?;
    acquire(role, player, resume, Instant::now())?;
    let summary = role_summary(role, Some(player));
    let side = role.side;
    game.teams
        .get_or_insert_with(teams::Teams::default)
        .members
        .entry(player)
        .or_insert(teams::Team::for_side(side));
    Ok(Json(observers::SeatSummary::Operational(summary)))
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
) -> ApiResult<observers::SeatSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    if game.observers.contains_key(&role_id) {
        observers::validate(game, role_id, session.player_id, request.lease_generation)?;
        let seat = game.observers.get_mut(&role_id).unwrap();
        seat.occupancy.lease = Some(Lease::new(Instant::now()));
        return Ok(Json(observers::SeatSummary::Observer(observers::summary(
            seat,
            Some(session.player_id),
        ))));
    }
    validate_role_lease(game, role_id, session.player_id, request.lease_generation)?;
    let role = game.roles.get_mut(&role_id).unwrap();
    role.lease = Some(Lease::new(Instant::now()));
    Ok(Json(observers::SeatSummary::Operational(role_summary(
        role,
        Some(session.player_id),
    ))))
}
pub(super) async fn release_role(
    Path((game_id, role_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(request): AuthJson<LeaseRequest>,
) -> ApiResult<observers::SeatSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    expire(game, Instant::now());
    if let Some(seat) = game.observers.get_mut(&role_id) {
        if seat.occupancy.owner != Some(session.player_id)
            || seat.occupancy.lease_generation != request.lease_generation
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "invalid_role_lease",
                "Role lease no longer valid",
            ));
        }
        release(role_id, &mut seat.occupancy);
        return Ok(Json(observers::SeatSummary::Observer(observers::summary(
            seat,
            Some(session.player_id),
        ))));
    }
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
    release(role_id, role);
    Ok(Json(observers::SeatSummary::Operational(role_summary(
        role,
        Some(session.player_id),
    ))))
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

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn restore_preserves_absolute_deadlines_and_never_renews_expired_reservations() {
        let now = unix_millis();
        for (active, reserved, expected) in [
            (now - 2000, now - 1000, LeaseState::Available),
            (now - 1000, now + 60_000, LeaseState::Reserved),
            (now + 60_000, now + 120_000, LeaseState::Active),
        ] {
            let value = serde_json::json!({"active_until_unix_ms":active,"reserved_until_unix_ms":reserved});
            let lease: Lease = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(&lease).unwrap(), value);
            let role = Occupancy {
                owner: Some(Uuid::new_v4()),
                lease_generation: 3,
                lease: Some(lease),
            };
            assert_eq!(summary(&role, Instant::now()).0, expected);
        }
        assert!(serde_json::from_value::<Lease>(
            serde_json::json!({"active_until_unix_ms":2,"reserved_until_unix_ms":1})
        )
        .is_err());
    }
}
