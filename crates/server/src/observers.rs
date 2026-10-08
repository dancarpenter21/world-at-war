//! Scenario-owned read-only seats. No observer belongs to the authority graph.
use super::*;
use sim_scenario::{ObserverKind, ObserverSeatDefinition};

pub(super) struct ObserverSeat {
    pub definition: ObserverSeatDefinition,
    pub granted_player: Option<Uuid>,
    pub occupancy: leases::Occupancy,
}

pub(super) fn from_scenario(scenario: &Scenario) -> BTreeMap<Uuid, ObserverSeat> {
    scenario
        .observer_seats
        .iter()
        .map(|definition| {
            (
                definition.id,
                ObserverSeat {
                    definition: definition.clone(),
                    granted_player: None,
                    occupancy: leases::Occupancy::default(),
                },
            )
        })
        .collect()
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum SeatSummary {
    Operational(RoleSummary),
    Observer(ObserverSummary),
}

#[derive(Serialize)]
pub(super) struct ObserverSummary {
    id: Uuid,
    name: String,
    kind: ObserverKind,
    observer_kind: ObserverKind,
    held: bool,
    held_by_you: bool,
    claimable: bool,
    ai_controlled: bool,
    lease_generation: u64,
    lease_state: leases::LeaseState,
    lease_remaining_seconds: u64,
}

pub(super) fn summary(seat: &ObserverSeat, player: Option<Uuid>) -> ObserverSummary {
    let (lease_state, lease_remaining_seconds) =
        leases::summary(&seat.occupancy, std::time::Instant::now());
    let held = lease_state != leases::LeaseState::Available;
    ObserverSummary {
        id: seat.definition.id,
        name: seat.definition.name.clone(),
        kind: seat.definition.kind,
        observer_kind: seat.definition.kind,
        held,
        held_by_you: held && player.is_some() && seat.occupancy.owner == player,
        claimable: player.is_some() && seat.granted_player == player,
        ai_controlled: false,
        lease_generation: seat.occupancy.lease_generation,
        lease_state,
        lease_remaining_seconds,
    }
}

fn host(game: &Game, player: Uuid) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if game.host != player {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "host_required",
            "Only the host can manage observer grants",
        ));
    }
    Ok(())
}

#[derive(Serialize)]
pub(super) struct Participant {
    player_id: Uuid,
    display_name: String,
    observer_seats: Vec<Uuid>,
}
pub(super) async fn participants(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    session: auth::Session,
) -> ApiResult<Vec<Participant>> {
    let games = state.games.read().await;
    let game = games
        .get(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    host(game, session.player_id)?;
    Ok(Json(
        game.members
            .iter()
            .map(|id| Participant {
                player_id: *id,
                display_name: game
                    .participant_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "Guest".into()),
                observer_seats: game
                    .observers
                    .values()
                    .filter(|s| s.granted_player == Some(*id))
                    .map(|s| s.definition.id)
                    .collect(),
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub(super) struct GrantRequest {
    target_player_id: Uuid,
}
pub(super) async fn grant(
    Path((id, seat)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(request): AuthJson<GrantRequest>,
) -> ApiResult<ObserverSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    host(game, session.player_id)?;
    if !game.members.contains(&request.target_player_id) {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "join_required",
            "The guest must join this game first",
        ));
    }
    change_grant(
        game,
        seat,
        Some(request.target_player_id),
        session.player_id,
    )
}
pub(super) async fn revoke(
    Path((id, seat)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_): AuthJson<serde_json::Value>,
) -> ApiResult<ObserverSummary> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    host(game, session.player_id)?;
    change_grant(game, seat, None, session.player_id)
}
fn change_grant(
    game: &mut Game,
    id: Uuid,
    player: Option<Uuid>,
    actor: Uuid,
) -> ApiResult<ObserverSummary> {
    let seat = game.observers.get_mut(&id).ok_or_else(|| {
        api_error(
            StatusCode::NOT_FOUND,
            "observer_not_found",
            "Scenario does not define this observer seat",
        )
    })?;
    if seat.granted_player != player {
        leases::release(id, &mut seat.occupancy);
        seat.granted_player = player;
        game.authority_events.push(AuthorityEvent {
            tick: game.simulation.tick(),
            kind: "observer_grant_changed".into(),
            detail: format!("host {actor} assigned observer {id} to {player:?}"),
        });
    }
    Ok(Json(summary(seat, Some(actor))))
}

pub(super) fn acquire(
    game: &mut Game,
    id: Uuid,
    player: Uuid,
    resume: bool,
) -> ApiResult<SeatSummary> {
    let seat = game.observers.get_mut(&id).unwrap();
    if seat.granted_player != Some(player) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "observer_grant_required",
            "The host must grant access to this observer seat",
        ));
    }
    leases::acquire_occupancy(
        id,
        &mut seat.occupancy,
        player,
        resume,
        std::time::Instant::now(),
    )?;
    Ok(Json(SeatSummary::Observer(summary(seat, Some(player)))))
}
pub(super) fn validate(
    game: &Game,
    id: Uuid,
    player: Uuid,
    generation: u64,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let valid = game.observers.get(&id).is_some_and(|seat| {
        seat.granted_player == Some(player)
            && seat.occupancy.owner == Some(player)
            && seat.occupancy.lease_generation == generation
            && leases::active(&seat.occupancy)
    });
    if !valid {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "observer_access_denied",
            "An active scenario-authorized observer lease is required",
        ));
    }
    Ok(())
}

pub(super) async fn truth(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<ProjectionQuery>,
) -> ApiResult<sim_core::truth::TruthProjection> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    validate(game, query.role_id, query.player_id, query.lease_generation)?;
    Ok(Json(game.simulation.truth_projection()))
}

pub(super) async fn stream(
    ws: WebSocketUpgrade,
    session: auth::Session,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<ProjectionQuery>,
) -> Result<axum::response::Response, (StatusCode, Json<ErrorResponse>)> {
    {
        let games = state.games.read().await;
        let game = games
            .get(&id)
            .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
        validate(game, query.role_id, query.player_id, query.lease_generation)?;
    }
    Ok(ws
        .on_upgrade(move |mut socket| async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                let bytes = {
                    let mut games = state.games.write().await;
                    games
                        .get_mut(&id)
                        .filter(|game| {
                            state.sessions.valid(&session)
                                && validate(
                                    game,
                                    query.role_id,
                                    query.player_id,
                                    query.lease_generation,
                                )
                                .is_ok()
                        })
                        .and_then(|game| {
                            serde_json::to_string(&game.simulation.truth_projection()).ok()
                        })
                };
                let Some(bytes) = bytes else {
                    let _ = socket
                        .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                            code: 1008,
                            reason: "Observer access ended".into(),
                        })))
                        .await;
                    return;
                };
                if socket.send(Message::Text(bytes.into())).await.is_err() {
                    return;
                }
            }
        })
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observer_leases_share_reservation_expiry_and_logout_without_command_authority() {
        let mut game = authority_tests::game_for(regional_campaign_scenario());
        let id = Uuid::from_u128(9002);
        let player = Uuid::from_u128(44);
        let actor = game.host;
        let _ = change_grant(&mut game, id, Some(player), actor).unwrap();
        let _ = acquire(&mut game, id, player, false).unwrap();
        let generation = game.observers[&id].occupancy.lease_generation;
        assert!(validate(&game, id, player, generation).is_ok());
        assert!(validate_role_lease(&game, id, player, generation).is_err());
        game.observers.get_mut(&id).unwrap().occupancy.lease =
            Some(leases::Lease::reserved_for_test());
        assert!(validate(&game, id, player, generation).is_err());
        let _ = acquire(&mut game, id, player, true).unwrap();
        assert!(game.observers[&id].occupancy.lease_generation > generation);
        game.status = GameStatus::Paused;
        leases::expire(
            &mut game,
            std::time::Instant::now() + Duration::from_secs(391),
        );
        assert!(game.observers[&id].occupancy.owner.is_none());
        assert!(acquire(&mut game, id, player, true).is_err());
        let _ = acquire(&mut game, id, player, false).unwrap();
        leases::release_player(&mut game, player);
        assert!(game.observers[&id].occupancy.owner.is_none());
    }
}
