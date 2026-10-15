// Authenticated HTTP integration tests, included by auth::tests.
async fn agent_call(
    state: &AppState,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let response = router(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn external_agents_are_role_scoped_revocable_and_idempotent_without_browser_credentials() {
    let mut fixture = fixture().await;
    let scenario = sim_scenario::ai_defense_exercise_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(), scenario);
    let state = &fixture.state;
    let host = new_guest(state).await;
    let stranger = new_guest(state).await;
    let created = team_call(
        state,
        &host,
        "POST",
        "/v1/games",
        json!({"scenario_id":"ai-defense-exercise.v1"}),
    )
    .await;
    let game_id: Uuid = serde_json::from_value(created["game"]["id"].clone()).unwrap();
    let prefix = format!("/v1/games/{game_id}");
    let role = Uuid::from_u128(20102);
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/agents/{role}"),
            &stranger.0,
            &stranger.1,
            json!({"mode":"external"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let configured = team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/agents/{role}"),
        json!({"mode":"external","timeout_ms":10000}),
    )
    .await;
    let token = configured["token"].as_str().unwrap();
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&game_id).unwrap();
        game.status = GameStatus::Running;
        crate::agents::advance(game, crate::agents::now_ms());
        assert!(!String::from_utf8(recovery::encode(game).unwrap())
            .unwrap()
            .contains(token));
    }
    let (status, view) = agent_call(
        state,
        "GET",
        &format!("{prefix}/agent/work"),
        token,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(view["work"]["projection"]["tracks"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!view.to_string().contains("Blue Exercise Aircraft"));
    assert_eq!(
        view,
        agent_call(
            state,
            "GET",
            &format!("{prefix}/agent/work"),
            token,
            Value::Null
        )
        .await
        .1
    );
    assert_eq!(
        agent_call(state, "GET", &format!("{prefix}/truth"), token, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let decision = json!({"protocol_version":1,"decision_id":Uuid::new_v4(),"observation_id":view["work"]["observation_id"],"assignment_generation":configured["assignment_generation"],"actions":[{"type":"order","unit_id":Uuid::from_u128(51),"kind":{"Move":{"north_mps":0,"east_mps":30}}},{"type":"order","unit_id":Uuid::from_u128(11),"kind":{"Move":{"north_mps":0,"east_mps":30}}}],"explanation":"Defend"});
    let (status, result) = agent_call(
        state,
        "POST",
        &format!("{prefix}/agent/decisions"),
        token,
        decision.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["results"][0]["accepted"], false);
    assert_eq!(result["results"][1]["accepted"], true);
    assert_eq!(
        result,
        agent_call(
            state,
            "POST",
            &format!("{prefix}/agent/decisions"),
            token,
            decision.clone()
        )
        .await
        .1
    );
    let mut conflict = decision.clone();
    conflict["explanation"] = json!("Changed");
    assert_eq!(
        agent_call(
            state,
            "POST",
            &format!("{prefix}/agent/decisions"),
            token,
            conflict
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let inspection = team_call(
        state,
        &host,
        "GET",
        &format!("{prefix}/agents"),
        Value::Null,
    )
    .await;
    assert!(inspection["history"].as_array().unwrap().is_empty());
    assert!(inspection["roles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["observation"].is_null()));
    team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/agents/{role}"),
        json!({"mode":"external"}),
    )
    .await;
    assert_eq!(
        agent_call(
            state,
            "GET",
            &format!("{prefix}/agent/work"),
            token,
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // Old decision IDs cannot be reused through a rotated assignment.
    let huge = json!({"padding":"x".repeat(70000)});
    assert_eq!(
        agent_call(
            state,
            "POST",
            &format!("{prefix}/agent/decisions"),
            token,
            huge
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn agent_timeout_pause_and_recovery_invalidate_work_without_stopping_simulation() {
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
    let role = Uuid::from_u128(20102);
    let configured = team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/agents/{role}"),
        json!({"mode":"external","timeout_ms":1000}),
    )
    .await;
    let token = configured["token"].as_str().unwrap();
    let old = {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        game.status = GameStatus::Running;
        crate::agents::advance(game, crate::agents::now_ms());
        drop(games);
        agent_call(
            state,
            "GET",
            &format!("{prefix}/agent/work"),
            token,
            Value::Null,
        )
        .await
        .1
    };
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        let now = crate::agents::now_ms() + 1001;
        let before = recovery::encode(game).unwrap();
        let mut replay = recovery::decode(&before).unwrap();
        let mut input = recovery::TickInput::capture(game);
        input.agent_time_ms = Some(now);
        crate::agents::advance(game, now);
        ai_orders::process_ai_orders(game);
        advance_game_tick(game);
        intents::record_execution_results(game);
        input.apply(&mut replay).unwrap();
        assert_eq!(
            recovery::encode(game).unwrap(),
            recovery::encode(&replay).unwrap()
        );
        assert_eq!(game.status, GameStatus::Running);
    }
    let inspect = team_call(
        state,
        &host,
        "GET",
        &format!("{prefix}/agents"),
        Value::Null,
    )
    .await;
    assert_eq!(
        inspect["roles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["role_id"] == role.to_string())
            .unwrap()["fallback"],
        true
    );
    let stale = json!({"protocol_version":1,"decision_id":Uuid::new_v4(),"observation_id":old["work"]["observation_id"],"assignment_generation":configured["assignment_generation"],"actions":[]});
    assert_eq!(
        agent_call(
            state,
            "POST",
            &format!("{prefix}/agent/decisions"),
            token,
            stale
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    team_call(state, &host, "POST", &format!("{prefix}/pause"), json!({})).await;
    assert!(agent_call(
        state,
        "GET",
        &format!("{prefix}/agent/work"),
        token,
        Value::Null
    )
    .await
    .1["work"]
        .is_null());
}

#[tokio::test]
async fn mixed_blue_agent_and_human_pilot_wait_for_delivered_authority_and_orders() {
    let mut fixture = fixture().await;
    let scenario = sim_scenario::combat_training_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(), scenario);
    let state = &fixture.state;
    let host = new_guest(state).await;
    let created = team_call(
        state,
        &host,
        "POST",
        "/v1/games",
        json!({"scenario_id":"training-engagement.v1"}),
    )
    .await;
    let id: Uuid = serde_json::from_value(created["game"]["id"].clone()).unwrap();
    let prefix = format!("/v1/games/{id}");
    let commander = Uuid::from_u128(20102);
    let pilot = Uuid::from_u128(20103);
    let held = claim_seat(state, &prefix, &pilot.to_string(), &host.0, &host.1).await;
    let configured = team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/agents/{commander}"),
        json!({"mode":"external"}),
    )
    .await;
    let token = configured["token"].as_str().unwrap();
    team_call(state, &host, "POST", &format!("{prefix}/start"), json!({})).await;
    let request_id = {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        advance_game_tick(game);
        let track = game
            .simulation
            .projection_for(Uuid::from_u128(11), Side::Blue)
            .tracks[0]
            .track_id;
        let intent = PlayerIntent {
            intent_id: Uuid::new_v4(),
            issuer_role: pilot,
            target: Uuid::from_u128(11),
            requested_tick: 2,
            kind: sim_core::OrderKind::Engage { track_id: track },
        };
        let result = intents::submit_player_intent(
            game,
            pilot,
            SubmitIntentRequest {
                player_id: host.2,
                lease_generation: held["lease_generation"].as_u64().unwrap(),
                intent,
            },
        )
        .unwrap();
        let SubmissionOutcome::PendingAuthority { request_id, .. } = result else {
            panic!("pilot must request authority")
        };
        crate::agents::advance(game, crate::agents::now_ms());
        request_id
    };
    let initial = agent_call(
        state,
        "GET",
        &format!("{prefix}/agent/work"),
        token,
        Value::Null,
    )
    .await
    .1;
    assert!(initial["work"]["authority_requests"]
        .as_array()
        .unwrap()
        .is_empty());
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        for _ in 0..30 {
            advance_game_tick(game);
            if matches!(
                game.authority_requests[&request_id].status,
                AuthorityRequestStatus::PendingHuman { .. }
            ) {
                break;
            }
        }
        assert!(
            matches!(game.authority_requests[&request_id].status,AuthorityRequestStatus::PendingHuman{role_id} if role_id==commander),
            "{:?}",
            game.authority_requests[&request_id].status
        );
    }
    let decision = json!({"protocol_version":1,"decision_id":Uuid::new_v4(),"observation_id":initial["work"]["observation_id"],"assignment_generation":configured["assignment_generation"],"actions":[{"type":"authority_decision","request_id":request_id,"approve":true}]});
    // A guessed request ID must not be usable if it was not in the observation.
    assert_eq!(
        agent_call(
            state,
            "POST",
            &format!("{prefix}/agent/decisions"),
            token,
            decision
        )
        .await
        .1["results"][0]["accepted"],
        false
    );
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        crate::agents::advance(game, crate::agents::now_ms());
    }
    let current = agent_call(
        state,
        "GET",
        &format!("{prefix}/agent/work"),
        token,
        Value::Null,
    )
    .await
    .1;
    assert_eq!(
        current["work"]["authority_requests"][0]["id"],
        request_id.to_string()
    );
    let decision = json!({"protocol_version":1,"decision_id":Uuid::new_v4(),"observation_id":current["work"]["observation_id"],"assignment_generation":configured["assignment_generation"],"actions":[{"type":"authority_decision","request_id":request_id,"approve":true}]});
    let result = agent_call(
        state,
        "POST",
        &format!("{prefix}/agent/decisions"),
        token,
        decision,
    )
    .await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1["results"][0]["accepted"], true, "{}", result.1);
    let mut games = state.games.write().await;
    let game = games.get_mut(&id).unwrap();
    assert!(!game.pending_deliveries.is_empty());
    for _ in 0..20 {
        advance_game_tick(game);
        intents::record_execution_results(game);
    }
    assert!(!matches!(
        game.authority_requests[&request_id].status,
        AuthorityRequestStatus::PendingHuman { .. }
    ));
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_external_agent_submission_and_token_survive_restart_without_reinference() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
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
    let role = Uuid::from_u128(20102);
    let configured = team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/agents/{role}"),
        json!({"mode":"external"}),
    )
    .await;
    let token = configured["token"].as_str().unwrap();
    team_call(state, &host, "POST", &format!("{prefix}/start"), json!({})).await;
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        let now = crate::agents::now_ms();
        let mut input = recovery::TickInput::capture(game);
        input.agent_time_ms = Some(now);
        crate::agents::advance(game, now);
        ai_orders::process_ai_orders(game);
        advance_game_tick(game);
        intents::record_execution_results(game);
        let change = recovery::Boundary::new(game, Some(input), None, "simulation_tick").unwrap();
        state
            .durability
            .store
            .as_ref()
            .unwrap()
            .commit(&[change], &mut games, None)
            .await
            .unwrap();
    }
    let work = agent_call(
        state,
        "GET",
        &format!("{prefix}/agent/work"),
        token,
        Value::Null,
    )
    .await
    .1;
    let decision = json!({"protocol_version":1,"decision_id":Uuid::new_v4(),"observation_id":work["work"]["observation_id"],"assignment_generation":configured["assignment_generation"],"actions":[{"type":"order","unit_id":Uuid::from_u128(11),"kind":{"Move":{"north_mps":0,"east_mps":33}}}]});
    let result = agent_call(
        state,
        "POST",
        &format!("{prefix}/agent/decisions"),
        token,
        decision.clone(),
    )
    .await;
    assert_eq!(result.0, StatusCode::OK);
    assert_eq!(result.1["results"][0]["accepted"], true);
    let (messages, submissions) = {
        let games = state.games.read().await;
        let game = &games[&id];
        (game.network_messages.len(), game.intent_submissions.len())
    };
    restart(state, &database).await.unwrap();
    assert!(agent_call(
        state,
        "GET",
        &format!("{prefix}/agent/work"),
        token,
        Value::Null
    )
    .await
    .1["work"]
        .is_null());
    assert_eq!(
        agent_call(
            state,
            "POST",
            &format!("{prefix}/agent/decisions"),
            token,
            decision
        )
        .await
        .1,
        result.1
    );
    {
        let games = state.games.read().await;
        let game = &games[&id];
        assert_eq!(game.status, GameStatus::Paused);
        assert_eq!(
            (game.network_messages.len(), game.intent_submissions.len()),
            (messages, submissions)
        );
    }
    state.games.write().await.clear();
    drop(fixture);
    database.close().await;
}
