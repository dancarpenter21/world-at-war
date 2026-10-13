// Real PostgreSQL integration tests. Each test creates and drops its own database.
use tokio_postgres::{Client, NoTls};

struct Database {
    admin: Client,
    url: String,
    name: String,
}
impl Database {
    async fn new() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "set TEST_DATABASE_URL to an isolated PostgreSQL server with CREATEDB permission",
        );
        let (admin, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let name = format!("waw_test_{}", Uuid::new_v4().simple());
        admin
            .batch_execute(&format!(
                "CREATE DATABASE {name} ENCODING 'UTF8' TEMPLATE template0"
            ))
            .await
            .unwrap();
        let mut url = reqwest::Url::parse(&url).unwrap();
        url.set_path(&name);
        Self {
            admin,
            url: url.to_string(),
            name,
        }
    }
    async fn client(&self) -> Client {
        let (client, connection) = tokio_postgres::connect(&self.url, NoTls).await.unwrap();
        tokio::spawn(async move {
            let _ = connection.await;
        });
        client
    }
    async fn close(self) {
        self.admin
            .batch_execute(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .await
            .unwrap();
    }
}
async fn durable_fixture(database: &Database) -> Fixture {
    let mut fixture = fixture().await;
    attach_store(&mut fixture.state, database).await;
    fixture
}
async fn attach_store(state: &mut AppState, database: &Database) {
    let store = Arc::new(storage::Store::connect(&database.url).await.unwrap());
    let mut sessions = Sessions::load(Some(store.clone())).await.unwrap();
    sessions.secure = true;
    state.sessions = Arc::new(sessions);
    state.durability = Arc::new(recovery::Durability::new(Some(store)));
}
async fn restart(state: &mut AppState, database: &Database) -> anyhow::Result<()> {
    state.sessions = Arc::new(Sessions::new().unwrap());
    state.durability = Arc::new(recovery::Durability::default());
    state.games = Arc::new(RwLock::new(BTreeMap::new()));
    // Wait until the dropped connection has released its advisory lock.
    for _ in 0..100 {
        match storage::Store::connect(&database.url).await {
            Ok(store) => {
                let store = Arc::new(store);
                let mut sessions = Sessions::load(Some(store.clone())).await?;
                sessions.secure = true;
                state.sessions = Arc::new(sessions);
                state.durability = Arc::new(recovery::Durability::new(Some(store)));
                return recovery::load(state).await;
            }
            Err(_) => tokio::task::yield_now().await,
        }
    }
    anyhow::bail!("runtime lock was not released")
}
async fn claim_seat(state: &AppState, prefix: &str, role: &str, cookie: &str, csrf: &str) -> Value {
    let (status, _, value) = call(
        state,
        "POST",
        &format!("{prefix}/roles/{role}/claim"),
        cookie,
        csrf,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_accounts_sessions_seats_and_logout_survive_restart() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, player) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    let held = claim_seat(state, &prefix, &role, &cookie, &csrf).await;
    assert!(
        storage::Store::connect(&database.url).await.is_err(),
        "second runtime must be fenced"
    );
    let (status, _, registered) = call(
        state,
        "POST",
        "/v1/auth/register",
        &cookie,
        &csrf,
        json!({"username":"Saved_Commander","password":"a long test password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{registered}");
    assert_eq!(registered["player_id"], player.to_string());
    assert_eq!(registered["username"], "saved_commander");
    let client = database.client().await;
    let row = client
        .query_one("SELECT password_hash FROM players WHERE id=$1", &[&player])
        .await
        .unwrap();
    assert!(row.get::<_, String>(0).starts_with("$argon2id$"));
    let key = hash(cookie.split_once('=').unwrap().1);
    assert!(client
        .query_opt(
            "SELECT token_hash FROM browser_sessions WHERE token_hash=$1",
            &[&key]
        )
        .await
        .unwrap()
        .is_some());
    restart(state, &database).await.unwrap();
    let (status, _, session) =
        call(state, "GET", "/v1/auth/session", &cookie, "", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(session["username"], "saved_commander");
    let (_, _, roles) = call(
        state,
        "GET",
        &format!("{prefix}/roles"),
        &cookie,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(
        roles
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == role)
            .unwrap()["lease_generation"],
        held["lease_generation"]
    );
    let (status, headers, login) = call(
        state,
        "POST",
        "/v1/auth/login",
        &cookie,
        "",
        json!({"username":"saved_commander","password":"a long test password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{login}");
    let new_cookie = headers[SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let new_csrf = login["csrf_token"].as_str().unwrap();
    assert_ne!(new_cookie, cookie);
    assert_ne!(new_csrf, csrf);
    assert_eq!(
        call(state, "GET", "/v1/auth/session", &cookie, "", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            state,
            "POST",
            "/v1/auth/logout",
            &new_cookie,
            new_csrf,
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    restart(state, &database).await.unwrap();
    assert_eq!(
        call(
            state,
            "GET",
            "/v1/auth/session",
            &new_cookie,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let game_id = prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap();
    assert!(
        state.games.read().await[&game_id].roles[&role.parse().unwrap()]
            .owner
            .is_none()
    );
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn failed_durable_logout_rolls_back_session_and_seat_and_pauses_game() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, player) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    claim_seat(state, &prefix, &role, &cookie, &csrf).await;
    let client = database.client().await;
    client.batch_execute("CREATE FUNCTION reject_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected write failure'; END $$; CREATE TRIGGER fail_event BEFORE INSERT ON game_events FOR EACH ROW EXECUTE FUNCTION reject_event()").await.unwrap();
    assert_eq!(
        call(state, "POST", "/v1/auth/logout", &cookie, &csrf, json!({}))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(state, "GET", "/v1/auth/session", &cookie, "", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    let game_id = prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap();
    {
        let games = state.games.read().await;
        assert_eq!(games[&game_id].status, GameStatus::Paused);
        assert_eq!(
            games[&game_id].roles[&role.parse().unwrap()].owner,
            Some(player)
        );
        assert!(games[&game_id].operational_error.is_some());
    }
    assert_eq!(
        client
            .query_one("SELECT count(*) FROM browser_sessions", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    client
        .batch_execute("DROP TRIGGER fail_event ON game_events")
        .await
        .unwrap();
    restart(state, &database).await.unwrap();
    assert_eq!(
        call(state, "GET", "/v1/auth/session", &cookie, "", Value::Null)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        state.games.read().await[&game_id].roles[&role.parse().unwrap()].owner,
        Some(player)
    );
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn database_schema_and_snapshot_corruption_fail_explicitly() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    create_game(state, &cookie, &csrf).await;
    let client = database.client().await;
    client
        .batch_execute("UPDATE games SET snapshot_sha256='corrupt'")
        .await
        .unwrap();
    assert!(restart(state, &database)
        .await
        .unwrap_err()
        .to_string()
        .contains("checksum"));
    drop(fixture);
    client
        .batch_execute("UPDATE schema_migrations SET checksum='corrupt' WHERE version=1")
        .await
        .unwrap();
    assert!(storage::Store::connect(&database.url).await.is_err());
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_tick_journal_replays_pending_orders_and_archives_role_filtered_events() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    let held = claim_seat(state, &prefix, &role, &cookie, &csrf).await;
    let generation = held["lease_generation"].as_u64().unwrap();
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/start"),
            &cookie,
            &csrf,
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let game_id = prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap();
    let role_id: Uuid = role.parse().unwrap();
    let target = state.games.read().await[&game_id].roles[&role_id].command_units[0];
    let intent_id = Uuid::new_v4();
    let request = json!({"lease_generation":generation,"intent":{
        "intent_id":intent_id,"issuer_role":role,"target":target,
        "kind":{"kind":"move","north_mps":12.0,"east_mps":34.0},"requested_tick":0
    }});
    // Use the actual domain serialization so the fixture follows the public DTO.
    let intent = PlayerIntent {
        intent_id,
        issuer_role: role_id,
        target,
        kind: sim_core::OrderKind::Move {
            north_mps: 12.0,
            east_mps: 34.0,
        },
        requested_tick: 0,
    };
    let mut request = request;
    request["intent"] = serde_json::to_value(intent).unwrap();
    let intent_path = format!("{prefix}/roles/{role}/intent");
    let (status, _, submitted) =
        call(state, "POST", &intent_path, &cookie, &csrf, request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    // Crash while the accepted command is still queued; it must remain pending.
    restart(state, &database).await.unwrap();
    assert_eq!(
        state.games.read().await[&game_id].status,
        GameStatus::Paused
    );
    assert_eq!(
        call(state, "POST", &intent_path, &cookie, &csrf, request)
            .await
            .2,
        submitted,
        "retry must return the original receipt"
    );
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/start"),
            &cookie,
            &csrf,
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut expected_events = Vec::new();
    for _ in 0..63 {
        let mut games = state.games.write().await;
        let game = games.get_mut(&game_id).unwrap();
        let input = recovery::TickInput::capture(game);
        let replay_input = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        input.apply(game).unwrap();
        expected_events.extend(
            game.network_message_events
                .iter()
                .filter(|record| network_message_visible(record, &game.roles[&role_id]))
                .map(|record| network_record_for_role(record, &game.roles[&role_id])),
        );
        let change =
            recovery::Boundary::new(game, Some(replay_input), None, "simulation_tick").unwrap();
        state
            .durability
            .store
            .as_ref()
            .unwrap()
            .commit(&[change], &mut games, None)
            .await
            .unwrap();
        assert!(games[&game_id].network_message_events.is_empty());
    }
    let before = {
        let mut games = state.games.write().await;
        let game = games.get_mut(&game_id).unwrap();
        serde_json::to_value(game.simulation.snapshot()).unwrap()
    };
    let runs = state
        .durability
        .store
        .as_ref()
        .unwrap()
        .runs()
        .await
        .unwrap();
    assert!(
        !runs[0].events.is_empty(),
        "must replay events after the periodic checkpoint"
    );
    let client = database.client().await;
    let suffix = runs[0].events.last().unwrap();
    client
        .execute(
            "UPDATE game_events SET state_sha256='corrupt' WHERE game_id=$1 AND sequence=$2",
            &[&game_id, &(suffix.sequence as i64)],
        )
        .await
        .unwrap();
    assert!(restart(state, &database)
        .await
        .unwrap_err()
        .to_string()
        .contains("hash mismatch"));
    client
        .execute(
            "UPDATE game_events SET state_sha256=$3 WHERE game_id=$1 AND sequence=$2",
            &[&game_id, &(suffix.sequence as i64), &suffix.hash],
        )
        .await
        .unwrap();
    restart(state, &database).await.unwrap();
    assert_eq!(
        state.games.read().await[&game_id].status,
        GameStatus::Paused
    );
    assert_eq!(
        serde_json::to_value(state.games.read().await[&game_id].simulation.snapshot()).unwrap(),
        before
    );
    let events_path =
        format!("{prefix}/network/events?role_id={role}&lease_generation={generation}&limit=2");
    let mut cursor = 0;
    let mut archived = Vec::new();
    loop {
        let (status, _, page) = call(
            state,
            "GET",
            &format!("{events_path}&cursor={cursor}"),
            &cookie,
            "",
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{page}");
        archived.extend(page["events"].as_array().unwrap().iter().cloned());
        if let Some(next) = page["next_cursor"].as_u64() {
            assert!(next > cursor);
            cursor = next;
        } else {
            break;
        }
    }
    assert!(!archived.is_empty());
    for expected in expected_events {
        assert!(archived.contains(&serde_json::to_value(expected).unwrap()));
    }
    let (outsider, outsider_csrf, _) = new_guest(state).await;
    assert_eq!(
        call(
            state,
            "GET",
            &events_path,
            &outsider,
            &outsider_csrf,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let client = database.client().await;
    let counts = client
        .query_one(
            "SELECT (SELECT count(*) FROM network_messages), (SELECT count(*) FROM network_events)",
            &[],
        )
        .await
        .unwrap();
    assert!(
        counts.get::<_, i64>(1) > counts.get::<_, i64>(0),
        "lifecycle transitions must reference shared message content"
    );
    recovery::shutdown(state).await.unwrap();
    let runs = state
        .durability
        .store
        .as_ref()
        .unwrap()
        .runs()
        .await
        .unwrap();
    assert_eq!(runs[0].sequence, runs[0].snapshot_sequence);
    assert!(runs[0].events.is_empty());
    // Reject a corrupted authoritative head even when its snapshot is intact.
    client.batch_execute("UPDATE game_events SET state_sha256='corrupt' WHERE sequence=(SELECT max(sequence) FROM game_events)").await.unwrap();
    // Recovery just checkpointed, so corrupt the authoritative head instead.
    client
        .batch_execute("UPDATE games SET current_sha256='corrupt'")
        .await
        .unwrap();
    assert!(restart(state, &database).await.is_err());
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_concurrent_claims_and_revoked_requests_cannot_reacquire_a_seat() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (first, first_csrf, _) = new_guest(state).await;
    let (second, second_csrf, _) = new_guest(state).await;
    let (prefix, role) = create_game(state, &first, &first_csrf).await;
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/join"),
            &second,
            &second_csrf,
            json!({"display_name":"Other"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let path = format!("{prefix}/roles/{role}/claim");
    let (a, b) = tokio::join!(
        call(state, "POST", &path, &first, &first_csrf, json!({})),
        call(state, "POST", &path, &second, &second_csrf, json!({}))
    );
    assert_eq!(
        [a.0, b.0]
            .iter()
            .filter(|code| **code == StatusCode::OK)
            .count(),
        1
    );
    let (cookie, csrf) = if a.0 == StatusCode::OK {
        (first, first_csrf)
    } else {
        (second, second_csrf)
    };
    {
        let gate = state.durability.gate.lock().await;
        // Both requests authenticate while the same session is valid. The later
        // claim must be rechecked after the earlier logout commits under the gate.
        let logout = call(state, "POST", "/v1/auth/logout", &cookie, &csrf, json!({}));
        let claim = call(state, "POST", &path, &cookie, &csrf, json!({}));
        tokio::pin!(logout);
        tokio::pin!(claim);
        assert!(futures_util::poll!(&mut logout).is_pending());
        assert!(futures_util::poll!(&mut claim).is_pending());
        drop(gate);
        assert_eq!(logout.as_mut().await.0, StatusCode::OK);
        assert_eq!(claim.as_mut().await.0, StatusCode::UNAUTHORIZED);
    }
    restart(state, &database).await.unwrap();
    let game_id = prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap();
    assert!(
        state.games.read().await[&game_id].roles[&role.parse().unwrap()]
            .owner
            .is_none()
    );
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_observer_grants_survive_restart_but_expired_sessions_lose_their_seats() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (host, host_csrf, _) = new_guest(state).await;
    let (guest, guest_csrf, guest_id) = new_guest(state).await;
    let (status, _, created) = call(
        state,
        "POST",
        "/v1/games",
        &host,
        &host_csrf,
        json!({"scenario_id":"regional-campaign.v1"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let prefix = format!("/v1/games/{}", created["game"]["id"].as_str().unwrap());
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/join"),
            &guest,
            &guest_csrf,
            json!({"display_name":"Monitor"})
        )
        .await
        .0,
        StatusCode::OK
    );
    let role = Uuid::from_u128(9001);
    let grant = format!("{prefix}/roles/{role}/observer-grant");
    assert_eq!(
        call(
            state,
            "PUT",
            &grant,
            &host,
            &host_csrf,
            json!({"target_player_id":guest_id})
        )
        .await
        .0,
        StatusCode::OK
    );
    let held = claim_seat(state, &prefix, &role.to_string(), &guest, &guest_csrf).await;
    let generation = held["lease_generation"].as_u64().unwrap();
    let truth = format!("{prefix}/truth?role_id={role}&lease_generation={generation}");
    restart(state, &database).await.unwrap();
    assert_eq!(
        call(state, "GET", &truth, &guest, "", Value::Null).await.0,
        StatusCode::OK
    );
    let client = database.client().await;
    client
        .execute(
            "UPDATE browser_sessions SET expires_unix=1 WHERE player_id=$1",
            &[&guest_id],
        )
        .await
        .unwrap();
    restart(state, &database).await.unwrap();
    assert_eq!(
        call(state, "GET", &truth, &guest, "", Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    let row = client
        .query_one(
            "SELECT grant_id, owner_id, generation FROM game_seats WHERE seat_id=$1",
            &[&role],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<Uuid>>(0), Some(guest_id));
    assert_eq!(row.get::<_, Option<Uuid>>(1), None);
    assert!(row.get::<_, i64>(2) > generation as i64);
    assert_eq!(
        call(state, "DELETE", &grant, &host, &host_csrf, json!({}))
            .await
            .0,
        StatusCode::OK
    );
    restart(state, &database).await.unwrap();
    assert_eq!(
        client
            .query_one("SELECT grant_id FROM game_seats WHERE seat_id=$1", &[&role])
            .await
            .unwrap()
            .get::<_, Option<Uuid>>(0),
        None
    );
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_mutation_finishes_after_the_request_waiter_disconnects() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, player) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    let path = format!("{prefix}/roles/{role}/claim");
    {
        let gate = state.durability.gate.lock().await;
        let mut request = Box::pin(call(state, "POST", &path, &cookie, &csrf, json!({})));
        assert!(futures_util::poll!(&mut request).is_pending());
        tokio::task::yield_now().await;
        drop(request);
        drop(gate);
        let _finished = state.durability.gate.lock().await;
    }
    restart(state, &database).await.unwrap();
    let game_id = prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap();
    assert_eq!(
        state.games.read().await[&game_id].roles[&role.parse().unwrap()].owner,
        Some(player)
    );
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn performance_cycles_include_database_delay_and_reject_failed_commits() {
    let database = Database::new().await;
    let fixture = durable_fixture(&database).await;
    let state = &fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    claim_seat(state, &prefix, &role, &cookie, &csrf).await;
    let id: Uuid = prefix.rsplit('/').next().unwrap().parse().unwrap();
    let client = database.client().await;
    client.batch_execute("CREATE FUNCTION perf_delay() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.05); RETURN NEW; END $$; CREATE TRIGGER perf_delay BEFORE INSERT ON game_events FOR EACH ROW EXECUTE FUNCTION perf_delay();").await.unwrap();
    let path = std::env::temp_dir().join(format!("waw-durable-timing-{}.jsonl", Uuid::new_v4()));
    let writer = performance::start_test_trace(&path);
    state.games.write().await.get_mut(&id).unwrap().status = GameStatus::Running;
    let task = tokio::spawn(run_simulation_loop(state.clone()));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if state.games.read().await[&id].simulation.tick() > 0 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    client.batch_execute("CREATE OR REPLACE FUNCTION perf_delay() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected failure'; END $$;").await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !state.durability.failed.load(std::sync::atomic::Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    task.abort();
    let _ = task.await;
    writer.finish().unwrap();
    let rows: Vec<Value> = std::fs::read_to_string(&path).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    std::fs::remove_file(path).unwrap();
    let cycles: Vec<_> = rows.iter().filter(|row| row["kind"] == "server_cycle" && row["running_games"].as_array().is_some_and(|ids| ids.contains(&json!(id)))).collect();
    assert!(cycles.iter().any(|row| row["success"] == true && row["duration_ms"].as_f64().unwrap() >= 50.0));
    assert_eq!(cycles.last().unwrap()["success"], false);
    assert!(cycles.last().unwrap()["frame_progress"].is_null(), "rolled-back frames must never publish committed progress");
    let successful: Vec<_> = cycles.iter().filter(|row| row["success"] == true).collect();
    assert!(successful.iter().all(|row| row["frame_progress"].as_array().unwrap().iter().any(|p| p["game_id"] == json!(id) && p["frames"] == 1)));
    assert!(state.games.read().await[&id].diagnostics.last_committed_sample_absent());
    let commits: Vec<_> = rows.iter().filter(|row| row["kind"] == "persistence_commit" && row["game_ids"].as_array().is_some_and(|ids| ids.contains(&json!(id)))).collect();
    assert!(commits.iter().any(|row| row["success"] == true && row["duration_ms"].as_f64().unwrap() >= 50.0));
    assert_eq!(commits.last().unwrap()["success"], false);
    let ordinary: Vec<_> = commits.iter().filter(|row| row["success"] == true && row["checkpoints"] == 0).collect();
    assert!(!ordinary.is_empty());
    assert!(ordinary.iter().all(|row| row["snapshot_submitted_bytes"] == 0),
        "ordinary frame commits must not submit unused snapshots");
    assert_eq!(state.games.read().await[&id].status, GameStatus::Paused);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_ticks_preserve_unchanged_seat_rows_and_the_previous_checkpoint() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    claim_seat(state, &prefix, &role, &cookie, &csrf).await;
    let id: Uuid = prefix.rsplit('/').next().unwrap().parse().unwrap();
    let client = database.client().await;
    async fn versions(client: &Client) -> Vec<String> {
        client.query("SELECT 'seat:' || seat_id::text || ':' || xmin::text FROM game_seats UNION ALL SELECT 'member:' || player_id::text || ':' || xmin::text FROM game_memberships ORDER BY 1", &[])
            .await.unwrap().iter().map(|row| row.get(0)).collect()
    }
    let before_versions = versions(&client).await;
    let before_snapshot: Vec<u8> = client.query_one("SELECT snapshot FROM games WHERE id=$1", &[&id]).await.unwrap().get(0);
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        game.status = GameStatus::Running;
        let input = recovery::TickInput::capture(game);
        ai_orders::process_ai_orders(game);
        advance_game_tick(game);
        intents::record_execution_results(game);
        let change = recovery::Boundary::new(game, Some(input), None, "simulation_tick").unwrap();
        assert!(!change.checkpoint);
        state.durability.store.as_ref().unwrap().commit(&[change], &mut games, None).await.unwrap();
    }
    assert_eq!(versions(&client).await, before_versions, "unchanged rows must not generate new tuple versions");
    let row = client.query_one("SELECT snapshot,sequence,snapshot_sequence FROM games WHERE id=$1", &[&id]).await.unwrap();
    assert_eq!(row.get::<_, Vec<u8>>(0), before_snapshot);
    assert!(row.get::<_, i64>(1) > row.get::<_, i64>(2));
    restart(state, &database).await.unwrap();
    assert_eq!(state.games.read().await[&id].simulation.tick(), 1);
    assert_eq!(state.games.read().await[&id].status, GameStatus::Paused);
    database.close().await;
}
