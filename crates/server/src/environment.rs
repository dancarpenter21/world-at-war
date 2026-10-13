//! Audited controller interventions; operational roles receive local weather only.
use super::*;
use sim_core::sensing::{Environment, Weather, WeatherRegion};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Intervention {
    pub actor: Uuid,
    pub role_id: Uuid,
    pub revision: u64,
    pub effective_tick: u64,
    pub baseline: Weather,
    pub regions: Vec<WeatherRegion>,
}
#[derive(Deserialize)]
pub(super) struct Update {
    pub lease_generation: u64,
    pub expected_revision: u64,
    pub baseline: Weather,
    pub regions: Vec<WeatherRegion>,
}
fn controller(
    game: &Game,
    role: Uuid,
    player: Uuid,
    generation: u64,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    observers::validate(game, role, player, generation)?;
    if !matches!(
        game.observers[&role].definition.kind,
        sim_scenario::ObserverKind::Controller | sim_scenario::ObserverKind::WhiteCell
    ) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "controller_required",
            "Only a granted game controller can change weather",
        ));
    }
    Ok(())
}
pub(super) async fn read(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<ProjectionQuery>,
) -> ApiResult<Environment> {
    let games = state.games.read().await;
    let game = games
        .get(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    observers::validate(game, query.role_id, query.player_id, query.lease_generation)?;
    let environment = game.simulation.environment().ok_or_else(|| {
        api_error(
            StatusCode::CONFLICT,
            "sensing_unavailable",
            "Migrate this game to the sensing rules first",
        )
    })?;
    Ok(Json(environment.clone()))
}
pub(super) async fn update(
    session: auth::Session,
    Path((id, role)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    AuthJson(request): AuthJson<Update>,
) -> ApiResult<Environment> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    controller(game, role, session.player_id, request.lease_generation)?;
    if game
        .simulation
        .environment()
        .is_some_and(|e| e.revision != request.expected_revision)
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "weather_revision",
            "Weather changed; refresh the preview",
        ));
    }
    game.simulation
        .schedule_weather(
            request.expected_revision,
            request.baseline.clone(),
            request.regions.clone(),
        )
        .map_err(|error| api_error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_weather", error))?;
    let environment = game.simulation.environment().unwrap().clone();
    game.environment_interventions.push(Intervention {
        actor: session.player_id,
        role_id: role,
        revision: environment.revision,
        effective_tick: game.simulation.tick() + 1,
        baseline: request.baseline,
        regions: request.regions,
    });
    Ok(Json(environment))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_and_monitor_do_not_inherit_weather_edit_privileges() {
        let mut game = authority_tests::game_from_scenario(regional_campaign_scenario());
        let player = game.host;
        let roles: Vec<_> = game.observers.keys().copied().collect();
        for id in roles {
            assert!(controller(&game, id, player, 0).is_err());
            let seat = game.observers.get_mut(&id).unwrap();
            seat.granted_player = Some(player);
            seat.occupancy.owner = Some(player);
            seat.occupancy.lease = Some(leases::Lease::new(std::time::Instant::now()));
            let expected = seat.definition.kind == sim_scenario::ObserverKind::Controller;
            assert_eq!(controller(&game, id, player, 0).is_ok(), expected);
            game.observers.get_mut(&id).unwrap().granted_player = None;
            assert!(controller(&game, id, player, 0).is_err());
        }
    }
}
