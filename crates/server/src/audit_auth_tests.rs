#[tokio::test]
async fn audit_api_requires_current_operational_lease_and_exposes_only_local_metadata() {
    let fixture = fixture().await;
    let state = &fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    let held = claim_seat(state, &prefix, &role, &cookie, &csrf).await;
    let generation = held["lease_generation"].as_u64().unwrap();
    let path = format!("{prefix}/network/audit?role_id={role}&lease_generation={generation}");
    assert_eq!(
        call(state, "GET", &path, "", "", Value::Null).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/network/audit?role_id={role}&lease_generation=0"),
            &cookie,
            &csrf,
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let game_id = prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap();
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&game_id).unwrap();
        let terminal = game.roles[&role.parse().unwrap()].location_unit_id;
        for (kind, observer) in [
            (sim_core::audit::Kind::Scheduled, terminal),
            (sim_core::audit::Kind::Delivered, Uuid::from_u128(998877)),
        ] {
            game.audit.pending.push(sim_core::audit::Event {
                physical: None,
                kind,
                at_ns: 1,
                packet_id: Some(5),
                bytes: 30,
                context: Default::default(),
                terminal: Some(observer),
                reason: None,
            });
        }
    }
    let (status, _, page) = call(state, "GET", &path, &cookie, &csrf, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["events"].as_array().unwrap().len(), 1);
    assert_eq!(page["events"][0]["kind"], "scheduled");
    assert_eq!(page["events"][0]["sequence"], 1);
    for key in [
        "from",
        "to",
        "payload",
        "route",
        "origin_entity",
        "recipient_entity",
    ] {
        assert!(page["events"][0].get(key).is_none());
    }
    let (other_cookie, other_csrf, _) = new_guest(state).await;
    assert_eq!(
        call(state, "GET", &path, &other_cookie, &other_csrf, Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_audit_survives_recovery_and_retention_protects_pending_packets() {
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
    let id: Uuid = prefix.rsplit('/').next().unwrap().parse().unwrap();
    let role_id: Uuid = role.parse().unwrap();
    let target = state.games.read().await[&id].roles[&role_id].command_units[0];
    let intent = PlayerIntent {
        intent_id: Uuid::new_v4(),
        issuer_role: role_id,
        target,
        kind: sim_core::OrderKind::Move {
            north_mps: 12.,
            east_mps: 34.,
        },
        requested_tick: 0,
    };
    let request = json!({"lease_generation":generation,"intent":intent});
    let intent_path = format!("{prefix}/roles/{role}/intent");
    assert_eq!(
        call(state, "POST", &intent_path, &cookie, &csrf, request)
            .await
            .0,
        StatusCode::OK
    );
    let path = format!("{prefix}/network/audit?role_id={role}&lease_generation={generation}");
    let before = call(state, "GET", &path, &cookie, &csrf, Value::Null)
        .await
        .2;
    assert!(before["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "scheduled"));
    let client = database.client().await;
    client.batch_execute("UPDATE communications_audit SET recorded_at=CURRENT_TIMESTAMP-INTERVAL '2 days'; UPDATE network_events SET recorded_at=CURRENT_TIMESTAMP-INTERVAL '2 days'").await.unwrap();
    {
        let games = state.games.read().await;
        assert_eq!(
            state
                .durability
                .store
                .as_ref()
                .unwrap()
                .cleanup_audit(&games, 1)
                .await
                .unwrap(),
            0
        );
    }
    restart(state, &database).await.unwrap();
    assert_eq!(
        before,
        call(state, "GET", &path, &cookie, &csrf, Value::Null)
            .await
            .2
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
    for _ in 0..70 {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        let input = recovery::TickInput::capture(game);
        let replay_input = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        input.apply(game).unwrap();
        let boundary =
            recovery::Boundary::new(game, Some(replay_input), None, "simulation_tick").unwrap();
        state
            .durability
            .store
            .as_ref()
            .unwrap()
            .commit(&[boundary], &mut games, None)
            .await
            .unwrap();
    }
    let page = call(state, "GET", &path, &cookie, &csrf, Value::Null)
        .await
        .2;
    assert!(page["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["kind"] == "transmission_started"));
    let raw_before: i64 = client
        .query_one("SELECT COUNT(*) FROM communications_audit", &[])
        .await
        .unwrap()
        .get(0);
    restart(state, &database).await.unwrap();
    assert_eq!(
        page,
        call(state, "GET", &path, &cookie, &csrf, Value::Null)
            .await
            .2
    );
    let raw_after: i64 = client
        .query_one("SELECT COUNT(*) FROM communications_audit", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(raw_before, raw_after, "recovery must not rewrite history");
    client.batch_execute("UPDATE communications_audit SET recorded_at=CURRENT_TIMESTAMP-INTERVAL '2 days'; UPDATE network_events SET recorded_at=CURRENT_TIMESTAMP-INTERVAL '2 days'").await.unwrap();
    {
        let games = state.games.read().await;
        let store = state.durability.store.as_ref().unwrap();
        assert_eq!(
            store.cleanup_audit(&games, 0).await.unwrap_or(0),
            0,
            "disabled retention must not delete history"
        );
        assert!(store.cleanup_audit(&games, 1).await.unwrap() > 0);
    }
    let cleaned = call(state, "GET", &path, &cookie, &csrf, Value::Null)
        .await
        .2;
    assert_eq!(cleaned["truncated"], true);
    assert!(cleaned["events"].as_array().unwrap().is_empty());
    let lifecycle = call(
        state,
        "GET",
        &format!("{prefix}/network/events?role_id={role}&lease_generation={generation}"),
        &cookie,
        &csrf,
        Value::Null,
    )
    .await
    .2;
    assert_eq!(lifecycle["truncated"], true);
    restart(state, &database).await.unwrap();
    assert_eq!(
        cleaned,
        call(state, "GET", &path, &cookie, &csrf, Value::Null)
            .await
            .2
    );
    let journal: i64 = client
        .query_one("SELECT COUNT(*) FROM game_events", &[])
        .await
        .unwrap()
        .get(0);
    assert!(journal > 70);
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn audit_insert_failure_rolls_back_command_and_public_history() {
    for table in [
        "communications_audit",
        "communications_audit_roles",
        "communications_audit_runtime",
    ] {
        assert_audit_failure_rolls_back(table).await;
    }
}

async fn assert_audit_failure_rolls_back(table: &str) {
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
    let id: Uuid = prefix.rsplit('/').next().unwrap().parse().unwrap();
    let role_id: Uuid = role.parse().unwrap();
    let target = state.games.read().await[&id].roles[&role_id].command_units[0];
    let client = database.client().await;
    client
        .batch_execute(&format!(
            "ALTER TABLE {table} ADD CONSTRAINT reject_audit CHECK(false) NOT VALID"
        ))
        .await
        .unwrap();
    let intent = PlayerIntent {
        intent_id: Uuid::new_v4(),
        issuer_role: role_id,
        target,
        kind: sim_core::OrderKind::Move {
            north_mps: 1.,
            east_mps: 2.,
        },
        requested_tick: 0,
    };
    let response = call(
        state,
        "POST",
        &format!("{prefix}/roles/{role}/intent"),
        &cookie,
        &csrf,
        json!({"lease_generation":generation,"intent":intent}),
    )
    .await;
    assert_eq!(
        response.0,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        response.2
    );
    assert_eq!(state.games.read().await[&id].status, GameStatus::Paused);
    assert!(state.games.read().await[&id].network_messages.is_empty());
    let count: i64 = client
        .query_one("SELECT COUNT(*) FROM communications_audit", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0);
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn communications_audit_migration_upgrades_the_previous_schema_without_rewriting_it() {
    let database = Database::new().await;
    let client = database.client().await;
    client.batch_execute("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,checksum TEXT NOT NULL,applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP)").await.unwrap();
    for (index, sql) in [
        include_str!("../migrations/001_identity.sql"),
        include_str!("../migrations/002_games.sql"),
        include_str!("../migrations/003_network_audit.sql"),
        include_str!("../migrations/004_game_metadata.sql"),
        include_str!("../migrations/005_event_version.sql"),
    ]
    .into_iter()
    .enumerate()
    {
        client.batch_execute(sql).await.unwrap();
        client
            .execute(
                "INSERT INTO schema_migrations(version,checksum) VALUES($1,$2)",
                &[&((index + 1) as i32), &recovery::digest(sql.as_bytes())],
            )
            .await
            .unwrap();
    }
    let store = storage::Store::connect(&database.url).await.unwrap();
    let versions: i64 = client
        .query_one("SELECT COUNT(*) FROM schema_migrations", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(versions, 6);
    drop(store);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn archive_cleanup_is_bounded_transactional_and_keeps_shared_message_bodies() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    let (prefix, role) = create_game(state, &cookie, &csrf).await;
    let id: Uuid = prefix.rsplit('/').next().unwrap().parse().unwrap();
    let role: Uuid = role.parse().unwrap();
    {
        let mut games = state.games.write().await;
        let game = games.get_mut(&id).unwrap();
        game.status = GameStatus::Paused;
        let terminal = game.roles[&role].location_unit_id;
        let profile = game.message_profiles.keys().next().unwrap().clone();
        let message =
            transport::transmit_c2_message(game, role, terminal, &profile, "Local fixture".into())
                .unwrap();
        for _ in 0..3002 {
            game.audit.pending.push(sim_core::audit::Event {
                physical: None,
                kind: sim_core::audit::Kind::Delivered,
                at_ns: 0,
                packet_id: None,
                bytes: 0,
                context: sim_core::audit::Context {
                    message_id: Some(message.to_string()),
                    ..Default::default()
                },
                terminal: Some(terminal),
                reason: None,
            });
        }
        let change = recovery::Boundary::new(game, None, None, "local_fixture").unwrap();
        state
            .durability
            .store
            .as_ref()
            .unwrap()
            .commit(&[change], &mut games, None)
            .await
            .unwrap();
    }
    let client = database.client().await;
    client
        .batch_execute(
            "UPDATE communications_audit SET recorded_at=CURRENT_TIMESTAMP-INTERVAL '2 days'",
        )
        .await
        .unwrap();
    let games = state.games.read().await;
    let store = state.durability.store.as_ref().unwrap();
    assert_eq!(
        store.cleanup_audit(&games, 1).await.unwrap(),
        0,
        "fresh C2 history protects its older packet archive"
    );
    client
        .batch_execute("UPDATE network_events SET recorded_at=CURRENT_TIMESTAMP-INTERVAL '2 days'")
        .await
        .unwrap();
    client.batch_execute("CREATE FUNCTION reject_cleanup() RETURNS TRIGGER LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test cleanup failure'; END $$; CREATE TRIGGER reject_cleanup BEFORE DELETE ON communications_audit FOR EACH ROW EXECUTE FUNCTION reject_cleanup()").await.unwrap();
    assert!(store.cleanup_audit(&games, 1).await.is_err());
    let floors: Value = client
        .query_one("SELECT floors FROM communications_audit_runtime", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        floors,
        json!({}),
        "failed deletion must roll back truncation markers"
    );
    client
        .batch_execute("DROP TRIGGER reject_cleanup ON communications_audit")
        .await
        .unwrap();
    assert_eq!(store.cleanup_audit(&games, 1).await.unwrap(), 1000);
    let bodies: i64 = client
        .query_one("SELECT COUNT(*) FROM network_messages", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        bodies, 1,
        "partial archive cleanup must preserve the shared body"
    );
    let backlog = store.audit_backlog(&games, 1).await.unwrap();
    assert!(backlog.0 > 0 && backlog.1 >= 172800);
    let before = recovery::encode(&games[&id]).unwrap();
    drop(games);
    let maintenance = tokio::spawn(audit::maintenance_for_test(state.clone(), 1));
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count: i64 = client
                .query_one("SELECT COUNT(*) FROM communications_audit", &[])
                .await
                .unwrap()
                .get(0);
            if count == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    maintenance.abort();
    let _ = maintenance.await;
    drained.expect("maintenance must drain multiple batches during the same sweep");
    let games = state.games.read().await;
    assert_eq!(
        before,
        recovery::encode(&games[&id]).unwrap(),
        "maintenance must not change paused gameplay state"
    );
    let bodies: i64 = client
        .query_one("SELECT COUNT(*) FROM network_messages", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(bodies, 0);
    assert_eq!(store.audit_backlog(&games, 1).await.unwrap(), (0, 0));
    drop(games);
    drop(fixture);
    database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn multi_game_audit_batch_publishes_heads_only_after_commit_and_recovers_once() {
    let database = Database::new().await;
    let mut fixture = durable_fixture(&database).await;
    let state = &mut fixture.state;
    let (cookie, csrf, _) = new_guest(state).await;
    let mut ids = Vec::new();
    let mut paths = Vec::new();
    for _ in 0..2 {
        let (prefix, role) = create_game(state, &cookie, &csrf).await;
        let held = claim_seat(state, &prefix, &role, &cookie, &csrf).await;
        paths.push(format!(
            "{prefix}/network/audit?role_id={role}&lease_generation={}",
            held["lease_generation"].as_u64().unwrap()
        ));
        ids.push(prefix.rsplit('/').next().unwrap().parse::<Uuid>().unwrap());
    }
    let client = database.client().await;
    let store = state.durability.store.as_ref().unwrap().clone();
    let journal_before: i64 = client
        .query_one("SELECT COUNT(*) FROM game_events", &[])
        .await
        .unwrap()
        .get(0);
    {
        let mut games = state.games.write().await;
        let mut changes = Vec::new();
        let mut heads = BTreeMap::new();
        for id in &ids {
            let game = games.get_mut(id).unwrap();
            heads.insert(*id, game.audit.heads.clone());
            let terminal = game.roles.values().next().unwrap().location_unit_id;
            game.audit.pending.push(sim_core::audit::Event {
                physical: None,
                kind: sim_core::audit::Kind::Attempt,
                at_ns: 1,
                packet_id: Some(77),
                bytes: 40,
                context: Default::default(),
                terminal: Some(terminal),
                reason: Some("quoted \"metadata\" \n unicode α".into()),
            });
            let mut second = game.audit.pending.last().unwrap().clone();
            second.packet_id = Some(78);
            game.audit.pending.push(second);
            let mut private = game.audit.pending.last().unwrap().clone();
            private.terminal = None;
            game.audit.pending.push(private);
            changes.push(recovery::Boundary::new(game, None, None, "batch_fixture").unwrap());
        }
        let pending = ids
            .iter()
            .map(|id| (*id, serde_json::to_value(&games[id].audit.pending).unwrap()))
            .collect::<BTreeMap<_, _>>();
        for table in [
            "communications_audit",
            "communications_audit_roles",
            "communications_audit_runtime",
        ] {
            client
                .batch_execute(&format!(
                    "ALTER TABLE {table} ADD CONSTRAINT reject_batch CHECK(false) NOT VALID"
                ))
                .await
                .unwrap();
            assert!(store.commit(&changes, &mut games, None).await.is_err());
            for id in &ids {
                assert_eq!(games[id].audit.heads, heads[id]);
                assert_eq!(
                    serde_json::to_value(&games[id].audit.pending).unwrap(),
                    pending[id]
                );
            }
            let count: i64 = client
                .query_one("SELECT COUNT(*) FROM communications_audit", &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(count, 0);
            let journal_after: i64 = client
                .query_one("SELECT COUNT(*) FROM game_events", &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                journal_after, journal_before,
                "audit failure rolls back every game journal boundary"
            );
            let roles_after: i64 = client
                .query_one("SELECT COUNT(*) FROM communications_audit_roles", &[])
                .await
                .unwrap()
                .get(0);
            assert_eq!(roles_after, 0);
            client
                .batch_execute(&format!("ALTER TABLE {table} DROP CONSTRAINT reject_batch"))
                .await
                .unwrap();
        }
        store.commit(&changes, &mut games, None).await.unwrap();
        for id in &ids {
            assert!(games[id].audit.pending.is_empty());
            assert_ne!(games[id].audit.heads, heads[id]);
        }
        // No-event commits preserve audit runtime tuple versions and retained coverage/floors.
        let versions = client
            .query(
                "SELECT game_id,xmin::TEXT FROM communications_audit_runtime ORDER BY game_id",
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1)))
            .collect::<Vec<_>>();
        let empty = ids
            .iter()
            .map(|id| {
                recovery::Boundary::new(games.get_mut(id).unwrap(), None, None, "empty_batch")
                    .unwrap()
            })
            .collect::<Vec<_>>();
        store.commit(&empty, &mut games, None).await.unwrap();
        let after = client
            .query(
                "SELECT game_id,xmin::TEXT FROM communications_audit_runtime ORDER BY game_id",
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, Uuid>(0), r.get::<_, String>(1)))
            .collect::<Vec<_>>();
        assert_eq!(versions, after);
    }
    let before = client.query("SELECT game_id,role_id,sequence,detail FROM communications_audit_roles ORDER BY game_id,role_id,sequence", &[]).await.unwrap().iter().map(|r| (r.get::<_,Uuid>(0),r.get::<_,Uuid>(1),r.get::<_,i64>(2),r.get::<_,Value>(3))).collect::<Vec<_>>();
    assert_eq!(
        before.len(),
        8,
        "two local events at each terminal serving two roles"
    );
    assert!(before
        .iter()
        .all(|r| r.3["reason"] == "quoted \"metadata\" \n unicode α"));
    let mut pages = Vec::new();
    for path in &paths {
        let first = call(
            state,
            "GET",
            &format!("{path}&limit=1"),
            &cookie,
            &csrf,
            Value::Null,
        )
        .await
        .2;
        assert_eq!(first["events"].as_array().unwrap().len(), 1);
        assert_eq!(first["events"][0]["packet_id"], 77);
        assert_eq!(first["next_cursor"], 1);
        let next = call(
            state,
            "GET",
            &format!("{path}&limit=1&cursor=1&packet_id=78&kind=attempt"),
            &cookie,
            &csrf,
            Value::Null,
        )
        .await
        .2;
        assert_eq!(next["events"][0]["sequence"], 2);
        assert!(next["next_cursor"].is_null());
        pages.push(first);
    }
    drop(store);
    restart(state, &database).await.unwrap();
    let after = client.query("SELECT game_id,role_id,sequence,detail FROM communications_audit_roles ORDER BY game_id,role_id,sequence", &[]).await.unwrap().iter().map(|r| (r.get::<_,Uuid>(0),r.get::<_,Uuid>(1),r.get::<_,i64>(2),r.get::<_,Value>(3))).collect::<Vec<_>>();
    assert_eq!(before, after);
    for (path, page) in paths.iter().zip(pages) {
        assert_eq!(
            call(
                state,
                "GET",
                &format!("{path}&limit=1"),
                &cookie,
                &csrf,
                Value::Null
            )
            .await
            .2,
            page
        );
    }
    for (id, role, _, _) in before {
        assert_eq!(state.games.read().await[&id].audit.heads[&role], 2);
    }
    drop(fixture);
    database.close().await;
}
