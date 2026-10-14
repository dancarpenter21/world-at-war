#[tokio::test]
async fn ridge_geometry_is_role_scoped_and_referee_revocation_removes_truth_access() {
    let mut fixture = fixture().await;
    let scenario = sim_scenario::ridge_relay_exercise_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(), scenario);
    let state = &fixture.state;
    let host = new_guest(state).await;
    let referee = new_guest(state).await;
    let stranger = new_guest(state).await;
    let created = team_call(
        state,
        &host,
        "POST",
        "/v1/games",
        json!({"scenario_id":"ridge-relay-exercise.v1"}),
    )
    .await;
    let prefix = format!("/v1/games/{}", created["game"]["id"].as_str().unwrap());
    let held = claim_seat(
        state,
        &prefix,
        &Uuid::from_u128(20102).to_string(),
        &host.0,
        &host.1,
    )
    .await;
    let role_auth = format!(
        "role_id={}&lease_generation={}",
        Uuid::from_u128(20102),
        held["lease_generation"]
    );
    let (_, _, picture) = call(
        state,
        "GET",
        &format!("{prefix}/network?{role_auth}"),
        &host.0,
        "",
        Value::Null,
    )
    .await;
    assert!(picture["links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["geometry_status"] == "terrain_blocked"));
    assert!(picture["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n["id"] != Uuid::from_u128(51).to_string()));
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/network?{role_auth}"),
            &stranger.0,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    team_join(state, &prefix, &referee, "Referee").await;
    let (seat, generation) = team_approve_white(state, &prefix, &host, &referee).await;
    let truth_url = format!("{prefix}/truth?role_id={seat}&lease_generation={generation}");
    let truth = team_call(state, &referee, "GET", &truth_url, Value::Null).await;
    assert!(truth["communication_links"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["geometry_status"] == "terrain_blocked"));
    team_call(
        state,
        &host,
        "DELETE",
        &format!("{prefix}/teams/members/{}", referee.2),
        json!({}),
    )
    .await;
    assert_eq!(
        call(state, "GET", &truth_url, &referee.0, "", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn ridge_pending_orders_recover_and_predecessor_campaign_keeps_its_rules() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let ridge = sim_scenario::ridge_relay_exercise_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(ridge.id.clone(), ridge);
    let prior_global = sim_scenario::upgrade_sensing(global_crisis_scenario());
    Arc::make_mut(&mut fixture.state.scenarios)
        .insert(prior_global.id.clone(), prior_global.clone());
    let state = &mut fixture.state;
    let host = new_guest(state).await;
    let created = team_call(
        state,
        &host,
        "POST",
        "/v1/games",
        json!({"scenario_id":"ridge-relay-exercise.v1"}),
    )
    .await;
    let prefix = format!("/v1/games/{}", created["game"]["id"].as_str().unwrap());
    let id = Uuid::parse_str(created["game"]["id"].as_str().unwrap()).unwrap();
    let role = Uuid::from_u128(20102);
    let held = claim_seat(state, &prefix, &role.to_string(), &host.0, &host.1).await;
    team_call(state, &host, "POST", &format!("{prefix}/start"), json!({})).await;
    let intent = PlayerIntent {
        intent_id: Uuid::new_v4(),
        issuer_role: role,
        target: Uuid::from_u128(11),
        kind: sim_core::OrderKind::Move {
            north_mps: 10.0,
            east_mps: 0.0,
        },
        requested_tick: 0,
    };
    team_call(
        state,
        &host,
        "POST",
        &format!("{prefix}/roles/{role}/intent"),
        json!({"lease_generation":held["lease_generation"],"intent":intent}),
    )
    .await;
    let before = serde_json::to_value(state.games.read().await[&id].simulation.snapshot()).unwrap();
    // Store a previously supported, sensing-enabled scenario without introducing a catalog dependency.
    let mut prior = authority_tests::game_for(prior_global.clone());
    prior.id = Uuid::new_v4();
    prior.host = host.2;
    prior.members.insert(host.2);
    prior.communications_catalog_checksum = state.communications_catalog.checksum();
    prior.message_pack_checksum = state.communications_catalog.message_pack_checksum();
    let prior_id = prior.id;
    let boundary = recovery::Boundary::new(&mut prior, None, None, "predecessor_fixture").unwrap();
    {
        let mut games = state.games.write().await;
        games.insert(prior_id, prior);
        state
            .durability
            .store
            .as_ref()
            .unwrap()
            .commit(&[boundary], &mut games, None)
            .await
            .unwrap();
    }
    Arc::make_mut(&mut state.scenarios).insert(
        prior_global.id.clone(),
        sim_scenario::upgrade_radio_propagation(prior_global),
    );
    restart(state, &database).await.unwrap();
    {
        let games = state.games.read().await;
        assert_eq!(
            serde_json::to_value(games[&id].simulation.snapshot()).unwrap(),
            before
        );
        assert!(!games[&prior_id].simulation.uses_radio_propagation());
        assert_eq!(games[&id].status, GameStatus::Paused);
    }
    team_call(state, &host, "POST", &format!("{prefix}/start"), json!({})).await;
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        for _ in 0..25 {
            advance_game_tick(game);
        }
        assert_eq!(
            game.simulation
                .projection_for(Uuid::from_u128(11), Side::Blue)
                .own_units
                .iter()
                .find(|u| u.id == Uuid::from_u128(11))
                .unwrap()
                .velocity
                .north_mps,
            10.0
        );
    }
    drop(fixture);
    database.close().await;
}
