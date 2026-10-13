//! Game teams and individually leased White Cell referees.
use super::*;
use sim_scenario::{ObserverKind, ObserverSeatDefinition};

type Failure = (StatusCode, Json<ErrorResponse>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Team {
    Blue,
    Red,
    WhiteCell,
}
impl Team {
    pub fn for_side(side: Side) -> Self {
        if side == Side::Red {
            Self::Red
        } else {
            Self::Blue
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RedControl {
    Human,
    Ai,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Teams {
    pub members: BTreeMap<Uuid, Team>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    former_members: BTreeSet<Uuid>,
    requests: BTreeMap<Uuid, TeamRequest>,
    notes: BTreeMap<Uuid, Note>,
    pub red_control: Option<RedControl>,
    pub control_revision: u64,
}
impl Teams {
    pub fn with_red_control(red_control: RedControl) -> Self {
        Self {
            red_control: Some(red_control),
            ..Self::default()
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct TeamRequest {
    pub id: Uuid,
    pub player_id: Uuid,
    pub team: Team,
    pub status: RequestStatus,
    tick: u64,
    decided_by: Option<Uuid>,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RequestStatus {
    Pending,
    Approved,
    Declined,
    Cancelled,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Note {
    id: Uuid,
    player_id: Uuid,
    display_name: String,
    tick: u64,
    text: String,
}
#[derive(Serialize)]
struct Member {
    player_id: Uuid,
    display_name: String,
    team: Option<Team>,
    referee_seat_id: Option<Uuid>,
    active: bool,
}
#[derive(Serialize)]
pub(super) struct View {
    team: Option<Team>,
    referee_seat_id: Option<Uuid>,
    can_manage: bool,
    can_control: bool,
    control_revision: u64,
    red_available: bool,
    requests: Vec<TeamRequest>,
    members: Vec<Member>,
}

pub(super) fn team(game: &Game, player: Uuid) -> Option<Team> {
    game.teams
        .as_ref()
        .and_then(|t| t.members.get(&player).copied())
}
fn seat_id(game: &Game, player: Uuid) -> Option<Uuid> {
    game.observers
        .values()
        .find(|s| s.definition.kind == ObserverKind::WhiteCell && s.granted_player == Some(player))
        .map(|s| s.definition.id)
}
pub(super) fn active_referee(game: &Game, player: Uuid) -> bool {
    team(game, player) == Some(Team::WhiteCell)
        && seat_id(game, player).is_some_and(|id| {
            let seat = &game.observers[&id];
            seat.occupancy.owner == Some(player) && leases::active(&seat.occupancy)
        })
}
pub(super) fn can_manage(game: &Game, player: Uuid) -> bool {
    game.host == player || active_referee(game, player)
}
pub(super) fn validate_referee(game: &Game, query: &ProjectionQuery) -> Result<(), Failure> {
    observers::validate(game, query.role_id, query.player_id, query.lease_generation)?;
    if game.observers[&query.role_id].definition.kind != ObserverKind::WhiteCell
        || team(game, query.player_id) != Some(Team::WhiteCell)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "white_cell_required",
            "An active White Cell referee seat is required",
        ));
    }
    Ok(())
}
pub(super) fn validate_side(game: &Game, player: Uuid, side: Side) -> Result<(), Failure> {
    if team(game, player).is_some_and(|t| t != Team::for_side(side))
        || game
            .teams
            .as_ref()
            .is_some_and(|t| t.former_members.contains(&player) && !t.members.contains_key(&player))
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "team_mismatch",
            "This role belongs to another team",
        ));
    }
    Ok(())
}
pub(super) fn red_available(game: &Game) -> bool {
    game.roles
        .values()
        .any(|r| r.side == Side::Red && r.claimable && !r.ai_controlled)
}
pub(super) fn human_red_supported(scenario: &Scenario) -> bool {
    matches!(
        scenario.id.as_str(),
        "global-crisis.v2" | "regional-campaign.v1"
    ) && scenario.authority.roles.iter().any(|r| r.side == Side::Red)
}
fn validate_destination(game: &Game, target: Team) -> Result<(), Failure> {
    if target == Team::Red && !red_available(game) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "red_unavailable",
            "Red is AI controlled or has no player roles",
        ));
    }
    if target == Team::Blue
        && !game
            .roles
            .values()
            .any(|r| r.side == Side::Blue && r.claimable && !r.ai_controlled)
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "blue_unavailable",
            "This scenario has no player Blue roles",
        ));
    }
    Ok(())
}
pub(super) fn assign(game: &mut Game, player: Uuid, target: Team) -> Result<(), Failure> {
    validate_destination(game, target)?;
    if team(game, player) == Some(target) {
        return Ok(());
    }
    leases::release_player(game, player);
    for seat in game
        .observers
        .values_mut()
        .filter(|s| s.granted_player == Some(player))
    {
        seat.granted_player = None;
    }
    game.teams
        .get_or_insert_with(Teams::default)
        .members
        .insert(player, target);
    if target == Team::WhiteCell {
        let id = Uuid::new_v5(&game.id, format!("white-cell:{player}").as_bytes());
        let seat = game
            .observers
            .entry(id)
            .or_insert_with(|| observers::ObserverSeat {
                definition: ObserverSeatDefinition {
                    id,
                    name: "White Cell referee".into(),
                    kind: ObserverKind::WhiteCell,
                },
                granted_player: None,
                occupancy: leases::Occupancy::default(),
            });
        seat.granted_player = Some(player);
    }
    Ok(())
}
pub(super) fn control(game: &Game, player: Uuid, expected: Option<u64>) -> Result<(), Failure> {
    if !can_manage(game, player) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "game_control_required",
            "Only the host or an active White Cell referee can control the game",
        ));
    }
    let revision = game.teams.as_ref().map_or(0, |t| t.control_revision);
    if expected.is_some_and(|r| r != revision) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "control_revision",
            "Game controls changed; refresh and try again",
        ));
    }
    Ok(())
}
pub(super) fn record_control(game: &mut Game, player: Uuid, detail: String) {
    if let Some(teams) = &mut game.teams {
        teams.control_revision += 1;
    }
    game.authority_events.push(AuthorityEvent {
        tick: game.simulation.tick(),
        kind: "referee_control".into(),
        detail: format!("player {player}: {detail}"),
    });
}
fn view(game: &Game, player: Uuid) -> View {
    let manager = can_manage(game, player);
    View {
        team: team(game, player),
        referee_seat_id: seat_id(game, player),
        can_manage: manager,
        can_control: manager,
        control_revision: game.teams.as_ref().map_or(0, |t| t.control_revision),
        red_available: red_available(game),
        requests: game
            .teams
            .as_ref()
            .map(|t| {
                t.requests
                    .values()
                    .filter(|r| manager || r.player_id == player)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default(),
        members: game
            .members
            .iter()
            .filter(|id| manager || **id == player)
            .map(|id| Member {
                player_id: *id,
                display_name: game
                    .participant_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| "Guest".into()),
                team: team(game, *id),
                referee_seat_id: seat_id(game, *id),
                active: active_referee(game, *id),
            })
            .collect(),
    }
}
fn participant(game: &Game, player: Uuid) -> Result<(), Failure> {
    if !game.members.contains(&player) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "join_required",
            "Join this game first",
        ));
    }
    Ok(())
}
pub(super) async fn read(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    session: auth::Session,
) -> ApiResult<View> {
    let games = state.games.read().await;
    let game = games
        .get(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    participant(game, session.player_id)?;
    Ok(Json(view(game, session.player_id)))
}
#[derive(Deserialize)]
pub(super) struct Request {
    pub request_id: Uuid,
    pub team: Team,
}
pub(super) async fn request(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(request): AuthJson<Request>,
) -> ApiResult<View> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    participant(game, session.player_id)?;
    validate_destination(game, request.team)?;
    if let Some(previous) = game
        .teams
        .as_ref()
        .and_then(|t| t.requests.get(&request.request_id))
    {
        if previous.player_id != session.player_id || previous.team != request.team {
            return Err(api_error(
                StatusCode::CONFLICT,
                "request_conflict",
                "Request ID was already used",
            ));
        }
        return Ok(Json(view(game, session.player_id)));
    }
    if game.teams.as_ref().is_some_and(|t| {
        t.requests
            .values()
            .any(|r| r.player_id == session.player_id && r.status == RequestStatus::Pending)
    }) {
        return Err(api_error(
            StatusCode::CONFLICT,
            "request_pending",
            "Cancel your pending team request first",
        ));
    }
    let holds_role = game
        .roles
        .values()
        .any(|r| r.owner == Some(session.player_id))
        || game
            .observers
            .values()
            .any(|s| s.occupancy.owner == Some(session.player_id));
    let immediate = game.host == session.player_id
        || (team(game, session.player_id).is_none()
            && !holds_role
            && !game
                .teams
                .as_ref()
                .is_some_and(|t| t.former_members.contains(&session.player_id))
            && request.team != Team::WhiteCell);
    if immediate {
        assign(game, session.player_id, request.team)?;
    }
    let tick = game.simulation.tick();
    game.teams
        .get_or_insert_with(Teams::default)
        .requests
        .insert(
            request.request_id,
            TeamRequest {
                id: request.request_id,
                player_id: session.player_id,
                team: request.team,
                status: if immediate {
                    RequestStatus::Approved
                } else {
                    RequestStatus::Pending
                },
                tick,
                decided_by: immediate.then_some(session.player_id),
            },
        );
    Ok(Json(view(game, session.player_id)))
}
#[derive(Deserialize)]
pub(super) struct Decision {
    pub approve: bool,
}
pub(super) async fn decide(
    Path((id, request_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(decision): AuthJson<Decision>,
) -> ApiResult<View> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    if !can_manage(game, session.player_id) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "referee_required",
            "Only the host or active referees can decide team requests",
        ));
    }
    let request = game
        .teams
        .as_ref()
        .and_then(|t| t.requests.get(&request_id))
        .cloned()
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "request_not_found",
                "Team request not found",
            )
        })?;
    if request.player_id == session.player_id && game.host != session.player_id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "another_referee_required",
            "Another referee must approve your transfer",
        ));
    }
    let target_status = if decision.approve {
        RequestStatus::Approved
    } else {
        RequestStatus::Declined
    };
    if request.status != RequestStatus::Pending {
        if request.status != target_status {
            return Err(api_error(
                StatusCode::CONFLICT,
                "request_decided",
                "This request has already been decided",
            ));
        }
        return Ok(Json(view(game, session.player_id)));
    }
    if decision.approve {
        assign(game, request.player_id, request.team)?;
    }
    let request = game
        .teams
        .as_mut()
        .unwrap()
        .requests
        .get_mut(&request_id)
        .unwrap();
    request.status = target_status;
    request.decided_by = Some(session.player_id);
    Ok(Json(view(game, session.player_id)))
}
pub(super) async fn cancel(
    Path((id, request_id)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_): AuthJson<serde_json::Value>,
) -> ApiResult<View> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    let request = game
        .teams
        .as_mut()
        .and_then(|t| t.requests.get_mut(&request_id))
        .filter(|r| r.player_id == session.player_id)
        .ok_or_else(|| {
            api_error(
                StatusCode::NOT_FOUND,
                "request_not_found",
                "Team request not found",
            )
        })?;
    if request.status == RequestStatus::Pending {
        request.status = RequestStatus::Cancelled;
    }
    Ok(Json(view(game, session.player_id)))
}
pub(super) async fn remove(
    Path((id, player)): Path<(Uuid, Uuid)>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(_): AuthJson<serde_json::Value>,
) -> ApiResult<View> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    if session.player_id != player && game.host != session.player_id {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "host_required",
            "Only the host can remove another team member",
        ));
    }
    leases::release_player(game, player);
    for seat in game
        .observers
        .values_mut()
        .filter(|s| s.granted_player == Some(player))
    {
        seat.granted_player = None;
    }
    if let Some(t) = &mut game.teams {
        t.members.remove(&player);
        t.former_members.insert(player);
        for request in t
            .requests
            .values_mut()
            .filter(|r| r.player_id == player && r.status == RequestStatus::Pending)
        {
            request.status = RequestStatus::Cancelled;
        }
    }
    Ok(Json(view(game, session.player_id)))
}

pub(super) async fn overview(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<ProjectionQuery>,
) -> ApiResult<serde_json::Value> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    validate_referee(game, &query)?;
    Ok(Json(serde_json::json!({
        "roles": game.roles.values().map(|r| role_summary(r, None)).collect::<Vec<_>>(),
        "notes": game.teams.as_ref().map(|t| t.notes.values().cloned().collect::<Vec<_>>()).unwrap_or_default(),
        "control_revision": game.teams.as_ref().map_or(0, |t| t.control_revision)
    })))
}
#[derive(Deserialize)]
pub(super) struct InspectionQuery {
    pub player_id: Uuid,
    pub role_id: Uuid,
    pub lease_generation: u64,
    pub target_role_id: Option<Uuid>,
    pub section: Option<String>,
}
pub(super) async fn inspect(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<InspectionQuery>,
) -> Result<axum::response::Response, Failure> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    validate_referee(
        game,
        &ProjectionQuery {
            player_id: query.player_id,
            role_id: query.role_id,
            lease_generation: query.lease_generation,
            after_sequence: None,
        },
    )?;
    if let Some(id) = query.target_role_id {
        let role =
            game.roles.get(&id).cloned().ok_or_else(|| {
                api_error(StatusCode::NOT_FOUND, "role_not_found", "Role not found")
            })?;
        #[derive(Serialize)]
        struct RoleInspection {
            role: RoleSummary,
            picture: sim_core::RoleProjection,
            planning: planning::PlanningView,
        }
        Ok(Json(RoleInspection {
            role: role_summary(&role, None),
            picture: game
                .simulation
                .projection_for(role.location_unit_id, role.side),
            planning: planning::view(game, &role),
        })
        .into_response())
    } else if query.section.as_deref() == Some("plans") {
        Ok(Json(
            serde_json::json!({ "plans": planning::white_cell_view(game), "orders": { "authority": game.authority_requests.values().collect::<Vec<_>>(), "submitted_intents": game.intent_submissions } }),
        ).into_response())
    } else {
        let messages = game.network_messages.iter().map(|record| serde_json::json!({
            "message": record.message, "state": record.state, "sequence": record.sequence,
            "delivered_at_ns": record.delivered_at_ns, "started_at_ns": record.started_at_ns,
            "terminal_at_ns": record.terminal_at_ns, "drop_reason": record.drop_reason
        })).collect::<Vec<_>>();
        Ok(Json(
            serde_json::json!({ "messages": messages, "history_limit": "Current retained message state; historical payloads not retained here are unavailable." }),
        ).into_response())
    }
}
#[derive(Deserialize)]
pub(super) struct NoteRequest {
    pub lease_generation: u64,
    pub role_id: Uuid,
    pub note_id: Uuid,
    pub text: String,
}
pub(super) async fn note(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    session: auth::Session,
    AuthJson(request): AuthJson<NoteRequest>,
) -> ApiResult<Note> {
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    validate_referee(
        game,
        &ProjectionQuery {
            player_id: session.player_id,
            role_id: request.role_id,
            lease_generation: request.lease_generation,
            after_sequence: None,
        },
    )?;
    if request.text.trim().is_empty() || request.text.chars().count() > 4000 {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_note",
            "Notes must contain 1–4000 characters",
        ));
    }
    let tick = game.simulation.tick();
    let display_name = game
        .participant_names
        .get(&session.player_id)
        .cloned()
        .unwrap_or_else(|| "Guest".into());
    let notes = &mut game.teams.as_mut().unwrap().notes;
    if let Some(note) = notes.get(&request.note_id) {
        if note.player_id != session.player_id || note.text != request.text.trim() {
            return Err(api_error(
                StatusCode::CONFLICT,
                "note_conflict",
                "Note ID was already used",
            ));
        }
        return Ok(Json(note.clone()));
    }
    let note = Note {
        id: request.note_id,
        player_id: session.player_id,
        display_name,
        tick,
        text: request.text.trim().into(),
    };
    notes.insert(note.id, note.clone());
    Ok(Json(note))
}
