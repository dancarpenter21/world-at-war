// Real authenticated router regressions, included by auth::tests.
type TeamPlayer = (String, String, Uuid);
async fn team_call(
    state: &AppState,
    player: &TeamPlayer,
    method: &str,
    path: &str,
    body: Value,
) -> Value {
    let (status, _, value) = call(state, method, path, &player.0, &player.1, body).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {value}");
    value
}
async fn team_join(state: &AppState, prefix: &str, player: &TeamPlayer, name: &str) {
    team_call(
        state,
        player,
        "POST",
        &format!("{prefix}/join"),
        json!({"display_name":name}),
    )
    .await;
}
async fn team_claim_white(state: &AppState, prefix: &str, player: &TeamPlayer) -> (Uuid, u64) {
    let view = team_call(
        state,
        player,
        "GET",
        &format!("{prefix}/teams"),
        Value::Null,
    )
    .await;
    let id: Uuid = serde_json::from_value(view["referee_seat_id"].clone()).unwrap();
    let seat = team_call(
        state,
        player,
        "POST",
        &format!("{prefix}/roles/{id}/claim"),
        json!({}),
    )
    .await;
    (id, seat["lease_generation"].as_u64().unwrap())
}
async fn team_approve_white(
    state: &AppState,
    prefix: &str,
    manager: &TeamPlayer,
    player: &TeamPlayer,
) -> (Uuid, u64) {
    let request = Uuid::new_v4();
    let pending = team_call(
        state,
        player,
        "POST",
        &format!("{prefix}/teams/requests"),
        json!({"request_id":request,"team":"white_cell"}),
    )
    .await;
    assert_eq!(
        pending["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == request.to_string())
            .unwrap()["status"],
        "pending"
    );
    team_call(
        state,
        manager,
        "POST",
        &format!("{prefix}/teams/requests/{request}/decision"),
        json!({"approve":true}),
    )
    .await;
    team_claim_white(state, prefix, player).await
}

#[tokio::test]
async fn white_cell_hosts_admits_multiple_referees_and_controls_the_shared_clock() {
    let fixture = fixture().await;
    let state = &fixture.state;
    let host = new_guest(state).await;
    let referee = new_guest(state).await;
    let pending = new_guest(state).await;
    let created = team_call(
        state,
        &host,
        "POST",
        "/v1/games",
        json!({"scenario_id":"command-link-exercise.v1","creator_team":"white_cell"}),
    )
    .await;
    let prefix = format!("/v1/games/{}", created["game"]["id"].as_str().unwrap());
    let (host_seat, host_generation) = team_claim_white(state, &prefix, &host).await;
    team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/start"),
        json!({"expected_control_revision":0}),
    )
    .await;
    for player in [&referee, &pending] {
        team_join(state, &prefix, player, "Late referee").await;
    }
    let (seat, generation) = team_approve_white(state, &prefix, &host, &referee).await;
    let auth = format!("role_id={seat}&lease_generation={generation}");
    let host_auth = format!("role_id={host_seat}&lease_generation={host_generation}");
    let request = Uuid::new_v4();
    team_call(
        state,
        &pending,
        "POST",
        &format!("{prefix}/teams/requests"),
        json!({"request_id":request,"team":"white_cell"}),
    )
    .await;
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/white-cell?{auth}"),
            &pending.0,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            state,
            "PUT",
            &format!("{prefix}/time-scale"),
            &pending.0,
            &pending.1,
            json!({"time_scale":5})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let speed = team_call(
        state,
        &referee,
        "PUT",
        &format!("{prefix}/time-scale"),
        json!({"time_scale":5,"expected_control_revision":1}),
    )
    .await;
    assert_eq!(speed["time_scale"], 5.0);
    assert_eq!(speed["status"], "running");
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/pause"),
            &host.0,
            &host.1,
            json!({"expected_control_revision":1})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    team_call(
        state,
        &referee,
        "POST",
        &format!("{prefix}/pause"),
        json!({"expected_control_revision":2}),
    )
    .await;
    let speed = team_call(
        state,
        &referee,
        "PUT",
        &format!("{prefix}/time-scale"),
        json!({"time_scale":0.25,"expected_control_revision":3}),
    )
    .await;
    assert_eq!(speed["status"], "paused");
    let note_id = Uuid::new_v4();
    let note = json!({"role_id":seat,"lease_generation":generation,"note_id":note_id,"text":"Private referee ruling"});
    for _ in 0..2 {
        team_call(
            state,
            &referee,
            "POST",
            &format!("{prefix}/white-cell/notes"),
            note.clone(),
        )
        .await;
    }
    let shared = team_call(
        state,
        &host,
        "GET",
        &format!("{prefix}/white-cell?{host_auth}"),
        Value::Null,
    )
    .await;
    assert_eq!(shared["notes"].as_array().unwrap().len(), 1);
    assert_eq!(shared["notes"][0]["text"], "Private referee ruling");
    // Pending players only see their own request; active referees can approve another member.
    let own = team_call(
        state,
        &pending,
        "GET",
        &format!("{prefix}/teams"),
        Value::Null,
    )
    .await;
    assert!(!own["can_manage"].as_bool().unwrap());
    assert_eq!(own["members"].as_array().unwrap().len(), 1);
    team_call(
        state,
        &referee,
        "POST",
        &format!("{prefix}/teams/requests/{request}/decision"),
        json!({"approve":false}),
    )
    .await;
    team_call(
        state,
        &host,
        "DELETE",
        &format!("{prefix}/teams/members/{}", referee.2),
        json!({}),
    )
    .await;
    for endpoint in [
        format!("{prefix}/white-cell?{auth}"),
        format!("{prefix}/truth?{auth}"),
        format!("{prefix}/white-cell/inspect?{auth}"),
    ] {
        assert_eq!(
            call(state, "GET", &endpoint, &referee.0, "", Value::Null)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/roles/{seat}/resume"),
            &referee.0,
            &referee.1,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    team_call(
        state,
        &host,
        "GET",
        &format!("{prefix}/white-cell?{host_auth}"),
        Value::Null,
    )
    .await;
    // Removing membership cannot be bypassed by leaving and directly claiming a combatant role.
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/roles/{}/claim", Uuid::from_u128(20102)),
            &referee.0,
            &referee.1,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn human_red_team_transfers_preserve_role_isolation_and_private_plans() {
    let fixture = fixture().await;
    let state = &fixture.state;
    let host = new_guest(state).await;
    let blue = new_guest(state).await;
    let red = new_guest(state).await;
    let created=team_call(state,&host,"POST","/v1/games",json!({"scenario_id":"regional-campaign.v1","creator_team":"white_cell","red_control":"human"})).await;
    let game_id: Uuid = serde_json::from_value(created["game"]["id"].clone()).unwrap();
    let prefix = format!("/v1/games/{game_id}");
    let (host_seat, host_generation) = team_claim_white(state, &prefix, &host).await;
    for (player, side) in [(&blue, "blue"), (&red, "red")] {
        team_join(state, &prefix, player, side).await;
        let result = team_call(
            state,
            player,
            "POST",
            &format!("{prefix}/teams/requests"),
            json!({"request_id":Uuid::new_v4(),"team":side}),
        )
        .await;
        assert_eq!(result["team"], side);
    }
    let blue_role = Uuid::from_u128(201);
    let red_role = Uuid::from_u128(251);
    let claimed = team_call(
        state,
        &blue,
        "POST",
        &format!("{prefix}/roles/{blue_role}/claim"),
        json!({}),
    )
    .await;
    let blue_generation = claimed["lease_generation"].as_u64().unwrap();
    team_call(
        state,
        &red,
        "POST",
        &format!("{prefix}/roles/{red_role}/claim"),
        json!({}),
    )
    .await;
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/roles/{blue_role}/claim"),
            &red.0,
            &red.1,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let roles = team_call(state, &red, "GET", &format!("{prefix}/roles"), Value::Null).await;
    assert!(roles
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r.get("side").is_some())
        .all(|r| r["side"] == "Red"));
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&game_id).unwrap();
        assert!(game
            .roles
            .values()
            .filter(|r| r.side == Side::Red)
            .all(|r| !r.ai_controlled && r.claimable));
        let mut draft = regional_campaign_scenario().campaign.unwrap();
        draft.name = "Unpublished private plan".into();
        game.planning.drafts.insert(blue_role, draft);
        game.status = GameStatus::Running;
        ai_orders::process_ai_orders(game);
        assert!(game.network_messages.is_empty());
    }
    // Human Red uses the same delivered order path, with no AI competing for its units.
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&game_id).unwrap();
        let target = Uuid::from_u128(61);
        let intent = PlayerIntent {
            intent_id: Uuid::new_v4(),
            issuer_role: red_role,
            target,
            kind: sim_core::OrderKind::Move {
                north_mps: 0.0,
                east_mps: 200.0,
            },
            requested_tick: game.simulation.tick() + 1,
        };
        let generation = game.roles[&red_role].lease_generation;
        assert!(intents::submit_player_intent(
            game,
            red_role,
            SubmitIntentRequest {
                player_id: red.2,
                lease_generation: generation,
                intent
            }
        )
        .is_ok());
        for _ in 0..10 {
            advance_game_tick(game);
        }
        let picture = game.simulation.projection_for(target, Side::Red);
        assert_eq!(
            picture
                .own_units
                .iter()
                .find(|u| u.id == target)
                .unwrap()
                .velocity
                .east_mps,
            200.0
        );
    }
    let host_auth = format!("role_id={host_seat}&lease_generation={host_generation}");
    let plans = team_call(
        state,
        &host,
        "GET",
        &format!("{prefix}/white-cell/inspect?{host_auth}&section=plans"),
        Value::Null,
    )
    .await;
    assert!(plans.to_string().contains("Unpublished private plan"));
    let blue_auth = format!("role_id={blue_role}&lease_generation={blue_generation}");
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/white-cell?{blue_auth}"),
            &blue.0,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let before = team_call(
        state,
        &blue,
        "GET",
        &format!("{prefix}/state?{blue_auth}"),
        Value::Null,
    )
    .await;
    let inspected = team_call(
        state,
        &host,
        "GET",
        &format!("{prefix}/white-cell/inspect?{host_auth}&target_role_id={blue_role}"),
        Value::Null,
    )
    .await;
    assert_eq!(inspected["picture"], before);
    assert_eq!(
        team_call(
            state,
            &blue,
            "GET",
            &format!("{prefix}/state?{blue_auth}"),
            Value::Null
        )
        .await,
        before
    );
    let (seat, generation) = team_approve_white(state, &prefix, &host, &blue).await;
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/state?{blue_auth}"),
            &blue.0,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let transfer = Uuid::new_v4();
    team_call(
        state,
        &blue,
        "POST",
        &format!("{prefix}/teams/requests"),
        json!({"request_id":transfer,"team":"red"}),
    )
    .await;
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/teams/requests/{transfer}/decision"),
            &blue.0,
            &blue.1,
            json!({"approve":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for _ in 0..2 {
        team_call(
            state,
            &host,
            "POST",
            &format!("{prefix}/teams/requests/{transfer}/decision"),
            json!({"approve":true}),
        )
        .await;
    }
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/truth?role_id={seat}&lease_generation={generation}"),
            &blue.0,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        teams::team(&state.games.read().await[&game_id], blue.2),
        Some(teams::Team::Red)
    );
}

#[tokio::test]
async fn white_cell_state_round_trips_without_changing_legacy_snapshot_bytes() {
    let fixture = fixture().await;
    let state = &fixture.state;
    let host = new_guest(state).await;
    let created = team_call(
        state,
        &host,
        "POST",
        "/v1/games",
        json!({"scenario_id":"command-link-exercise.v1"}),
    )
    .await;
    let id: Uuid = serde_json::from_value(created["game"]["id"].clone()).unwrap();
    let prefix = format!("/v1/games/{id}");
    let legacy = recovery::encode(&state.games.read().await[&id]).unwrap();
    assert!(!serde_json::from_slice::<Value>(&legacy).unwrap()["game"]
        .as_object()
        .unwrap()
        .contains_key("teams"));
    assert_eq!(
        recovery::encode(&recovery::decode(&legacy).unwrap()).unwrap(),
        legacy
    );
    team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/teams/requests"),
        json!({"request_id":Uuid::new_v4(),"team":"white_cell"}),
    )
    .await;
    let (seat, generation) = team_claim_white(state, &prefix, &host).await;
    team_call(state,&host,"POST",&format!("{prefix}/white-cell/notes"),json!({"role_id":seat,"lease_generation":generation,"note_id":Uuid::new_v4(),"text":"Saved team note"})).await;
    let bytes = recovery::encode(&state.games.read().await[&id]).unwrap();
    let restored = recovery::decode(&bytes).unwrap();
    assert_eq!(recovery::encode(&restored).unwrap(), bytes);
    assert!(teams::active_referee(&restored, host.2));
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["game"]["teams"]["notes"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        call(
            state,
            "POST",
            "/v1/games",
            &host.0,
            &host.1,
            json!({"scenario_id":"command-link-exercise.v1","red_control":"human"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL through TEST_DATABASE_URL"]
async fn saved_white_cell_members_notes_and_human_red_recover_paused() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let host = new_guest(&fixture.state).await;
    let referee = new_guest(&fixture.state).await;
    let created = team_call(&fixture.state, &host, "POST", "/v1/games", json!({"scenario_id":"regional-campaign.v1","creator_team":"white_cell","red_control":"human"})).await;
    let id: Uuid = serde_json::from_value(created["game"]["id"].clone()).unwrap();
    let prefix = format!("/v1/games/{id}");
    team_claim_white(&fixture.state, &prefix, &host).await;
    team_join(&fixture.state, &prefix, &referee, "Recovery referee").await;
    let (seat, generation) = team_approve_white(&fixture.state, &prefix, &host, &referee).await;
    team_call(&fixture.state, &referee, "POST", &format!("{prefix}/white-cell/notes"), json!({"role_id":seat,"lease_generation":generation,"note_id":Uuid::new_v4(),"text":"Resume referee review"})).await;
    team_call(
        &fixture.state,
        &referee,
        "PUT",
        &format!("{prefix}/time-scale"),
        json!({"time_scale":5,"expected_control_revision":0}),
    )
    .await;
    team_call(
        &fixture.state,
        &referee,
        "POST",
        &format!("{prefix}/start"),
        json!({"expected_control_revision":1}),
    )
    .await;
    restart(&mut fixture.state, &database).await.unwrap();
    let game = team_call(&fixture.state, &host, "GET", "/v1/games", Value::Null).await;
    assert_eq!(game[0]["status"], "paused");
    assert_eq!(game[0]["time_scale"], 5.0);
    assert_eq!(game[0]["red_control"], "human");
    let (new_seat, new_generation) = team_claim_white(&fixture.state, &prefix, &referee).await;
    assert_eq!(seat, new_seat);
    let view = team_call(
        &fixture.state,
        &referee,
        "GET",
        &format!("{prefix}/white-cell?role_id={seat}&lease_generation={new_generation}"),
        Value::Null,
    )
    .await;
    assert_eq!(view["notes"][0]["text"], "Resume referee review");
    assert!(fixture.state.games.read().await[&id]
        .roles
        .values()
        .filter(|r| r.side == Side::Red)
        .all(|r| !r.ai_controlled && r.claimable));
    drop(fixture);
    database.close().await;
}
