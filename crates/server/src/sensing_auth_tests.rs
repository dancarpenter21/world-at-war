#[tokio::test]
async fn host_speed_control_is_shared_validated_and_does_not_resume_paused_games() {
    let fixture=fixture().await;let state=&fixture.state;
    let (cookie,csrf,_)=new_guest(state).await;let (prefix,_)=create_game(state,&cookie,&csrf).await;
    let (guest,guest_csrf,_)=new_guest(state).await;
    assert_eq!(call(state,"PUT",&format!("{prefix}/time-scale"),&guest,&guest_csrf,json!({"time_scale":5})).await.0,StatusCode::FORBIDDEN);
    assert_eq!(call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":3})).await.0,StatusCode::BAD_REQUEST);
    assert_eq!(call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,"wrong",json!({"time_scale":5})).await.0,StatusCode::FORBIDDEN);
    call(state,"POST",&format!("{prefix}/pause"),&cookie,&csrf,json!({})).await;
    let (status,_,view)=call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":60})).await;
    assert_eq!(status,StatusCode::OK,"{view}");assert_eq!(view["time_scale"],60.0);assert_eq!(view["status"],"paused");
    let (_,_,games)=call(state,"GET","/v1/games",&guest,&guest_csrf,json!({})).await;assert_eq!(games[0]["time_scale"],60.0);
}

#[tokio::test]
async fn weather_edits_require_an_active_controller_and_preserve_revision_conflicts() {
    let mut fixture=fixture().await;
    let scenario=sim_scenario::sensing_exercise_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(),scenario);
    let state=&fixture.state;let (cookie,csrf,_)=new_guest(state).await;
    let (status,_,created)=call(state,"POST","/v1/games",&cookie,&csrf,json!({"scenario_id":"sensing-kernel-exercise.v1"})).await;
    assert_eq!(status,StatusCode::OK,"{created}");let id=created["game"]["id"].as_str().unwrap();let game_id=Uuid::parse_str(id).unwrap();
    let controller=state.games.read().await[&game_id].observers.values().find(|seat|seat.definition.kind==sim_scenario::ObserverKind::Controller).unwrap().definition.id;
    let update=json!({"lease_generation":0,"expected_revision":0,"baseline":sim_core::sensing::Weather::default(),"regions":[]});
    assert_eq!(call(state,"PUT",&format!("/v1/games/{id}/roles/{controller}/environment"),&cookie,&csrf,update.clone()).await.0,StatusCode::FORBIDDEN);
    let player=state.games.read().await[&game_id].host;
    assert_eq!(call(state,"PUT",&format!("/v1/games/{id}/roles/{controller}/observer-grant"),&cookie,&csrf,json!({"target_player_id":player})).await.0,StatusCode::OK);
    let held=claim_seat(state,&format!("/v1/games/{id}"),&controller.to_string(),&cookie,&csrf).await;
    let mut update=update;update["lease_generation"]=held["lease_generation"].clone();
    let (status,_,view)=call(state,"PUT",&format!("/v1/games/{id}/roles/{controller}/environment"),&cookie,&csrf,update.clone()).await;
    assert_eq!(status,StatusCode::OK,"{view}");assert_eq!(view["revision"],1);assert_eq!(view["pending"]["effective_tick"],1);
    assert_eq!(call(state,"PUT",&format!("/v1/games/{id}/roles/{controller}/environment"),&cookie,&csrf,update).await.0,StatusCode::CONFLICT);
    assert_eq!(state.games.read().await[&game_id].environment_interventions.len(),1);
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn sensing_migration_preserves_history_dry_runs_and_restarts_paused() {
    let database=Database::new().await;let mut fixture=durable_fixture(&database).await;let state=&mut fixture.state;
    let (cookie,csrf,_)=new_guest(state).await;let (prefix,_)=create_game(state,&cookie,&csrf).await;
    let id=Uuid::parse_str(prefix.rsplit('/').next().unwrap()).unwrap();
    assert_eq!(call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":5})).await.0,StatusCode::OK);
    let client=database.client().await;
    let history:Vec<(i64,String)>=client.query("SELECT sequence,state_sha256 FROM game_events WHERE game_id=$1 ORDER BY sequence",&[&id]).await.unwrap().iter().map(|r|(r.get(0),r.get(1))).collect();
    let head:String=client.query_one("SELECT current_sha256 FROM games WHERE id=$1",&[&id]).await.unwrap().get(0);
    let upgraded=state.scenarios.values().cloned().map(sim_scenario::upgrade_sensing).map(|s|(s.id.clone(),s)).collect();state.scenarios=Arc::new(upgraded);
    recovery::migrate_sensing(state,true).await.unwrap();
    assert_eq!(client.query_one("SELECT current_sha256 FROM games WHERE id=$1",&[&id]).await.unwrap().get::<_,String>(0),head);
    assert!(state.games.read().await[&id].simulation.sensing_configuration().is_none());
    recovery::migrate_sensing(state,false).await.unwrap();
    {let games=state.games.read().await;let game=&games[&id];assert_eq!(game.rules_version,Some(2));assert!(game.sensing_migration.is_some());assert_eq!(game.status,GameStatus::Paused);assert_eq!(game.time_scale,Some(5.0));}
    for (sequence,hash) in history {assert_eq!(client.query_one("SELECT state_sha256 FROM game_events WHERE game_id=$1 AND sequence=$2",&[&id,&sequence]).await.unwrap().get::<_,String>(0),hash);}
    restart(state,&database).await.unwrap();assert_eq!(state.games.read().await[&id].status,GameStatus::Paused);assert_eq!(state.games.read().await[&id].time_scale,Some(5.0));
    drop(client);drop(fixture);database.close().await;
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn sensing_migration_failure_rolls_back_all_game_heads() {
    let database=Database::new().await;let mut fixture=durable_fixture(&database).await;let state=&mut fixture.state;
    let (cookie,csrf,_)=new_guest(state).await;create_game(state,&cookie,&csrf).await;create_game(state,&cookie,&csrf).await;
    let client=database.client().await;
    let heads:Vec<(Uuid,String)>=client.query("SELECT id,current_sha256 FROM games ORDER BY id",&[]).await.unwrap().iter().map(|r|(r.get(0),r.get(1))).collect();
    state.scenarios=Arc::new(state.scenarios.values().cloned().map(sim_scenario::upgrade_sensing).map(|s|(s.id.clone(),s)).collect());
    client.batch_execute("CREATE FUNCTION reject_sensing_upgrade() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.scenario_version > OLD.scenario_version THEN RAISE EXCEPTION 'injected migration failure'; END IF; RETURN NEW; END $$; CREATE TRIGGER reject_sensing_upgrade BEFORE UPDATE ON game_metadata FOR EACH ROW EXECUTE FUNCTION reject_sensing_upgrade()").await.unwrap();
    assert!(recovery::migrate_sensing(state,false).await.is_err());
    for (id,hash) in heads {assert_eq!(client.query_one("SELECT current_sha256 FROM games WHERE id=$1",&[&id]).await.unwrap().get::<_,String>(0),hash);assert!(state.games.read().await[&id].simulation.sensing_configuration().is_none());}
    drop(client);drop(fixture);database.close().await;
}

#[tokio::test]
async fn radar_emission_command_waits_for_delivery_and_duplicate_retry_does_not_reexecute() {
    let mut fixture=fixture().await;let scenario=sim_scenario::upgrade_sensing(sensor_relay_exercise_scenario());
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(),scenario);
    let state=&fixture.state;let (cookie,csrf,_)=new_guest(state).await;
    let (_,_,created)=call(state,"POST","/v1/games",&cookie,&csrf,json!({"scenario_id":"sensor-relay-exercise.v1"})).await;
    let id=Uuid::parse_str(created["game"]["id"].as_str().unwrap()).unwrap();let prefix=format!("/v1/games/{id}");
    let role=state.games.read().await[&id].roles.values().find(|r|r.location_unit_id.as_u128()==11).unwrap().id;
    let held=claim_seat(state,&prefix,&role.to_string(),&cookie,&csrf).await;
    call(state,"POST",&format!("{prefix}/start"),&cookie,&csrf,json!({})).await;
    let intent_id=Uuid::from_u128(98100);let body=json!({"lease_generation":held["lease_generation"],"intent":{"intent_id":intent_id,"issuer_role":role,"target":Uuid::from_u128(11),"requested_tick":1,"kind":{"SetRadarEmission":{"emitting":false}}}});
    assert_eq!(call(state,"POST",&format!("{prefix}/roles/{role}/intent"),&cookie,&csrf,body.clone()).await.0,StatusCode::OK);
    {let mut games=state.games.write().await;let game=games.get_mut(&id).unwrap();assert!(game.simulation.projection_for(Uuid::from_u128(11),Side::Blue).sensing.unwrap().radar_emitting);for _ in 0..5 {advance_game_tick(game);intents::record_execution_results(game);}assert!(!game.simulation.projection_for(Uuid::from_u128(11),Side::Blue).sensing.unwrap().radar_emitting);}
    assert_eq!(call(state,"POST",&format!("{prefix}/roles/{role}/intent"),&cookie,&csrf,body).await.0,StatusCode::OK);
    assert_eq!(state.games.read().await[&id].intent_submissions.len(),1);
}

#[test]
fn upgraded_sensor_reports_fragment_and_do_not_publish_receiver_knowledge_before_reassembly() {
    let scenario=sim_scenario::upgrade_sensing(sensor_relay_exercise_scenario());
    let mut game=authority_tests::game_from_scenario(scenario);
    let post=Uuid::from_u128(5);let source=Uuid::from_u128(11);
    advance_game_tick(&mut game);
    assert!(!game.simulation.projection_for(source,Side::Blue).tracks.is_empty());
    assert!(game.simulation.projection_for(post,Side::Blue).tracks.is_empty());
    let record=game.network_messages.iter().find(|record|record.message.fields.contains_key("track")).unwrap().clone();
    assert_eq!(record.packet_id,None);assert!(record.encoded_bytes.len()>1200);assert_eq!(record.state,MessageState::Queued);
    let receiver=game.roles.values().find(|role|role.location_unit_id==post).unwrap().clone();
    assert!(!network_message_visible(&record,&receiver));
    let mut delivered=false;
    for _ in 0..40 {advance_game_tick(&mut game);if !game.simulation.projection_for(post,Side::Blue).tracks.is_empty(){delivered=true;break;}}
    assert!(delivered,"complete report must arrive through the existing fragment transport");
    let report=game.network_messages.iter().find(|record|record.delivered_at_ns.is_some() && record.message.fields.contains_key("track")).unwrap();
    assert!(network_message_visible(report,&receiver));
    if report.state==MessageState::Delivered {let sender=game.roles.get(&report.message.header.origin_role_id).unwrap();assert_eq!(network_record_for_role(report,sender).state,MessageState::InTransit);}
}

#[tokio::test]
async fn paused_clock_maintenance_releases_expired_sessions_and_duplicate_speed_does_not_reset_deadlines() {
    let fixture=fixture().await;let state=&fixture.state;let(cookie,csrf,player)=new_guest(state).await;let(prefix,role)=create_game(state,&cookie,&csrf).await;
    claim_seat(state,&prefix,&role,&cookie,&csrf).await;
    call(state,"POST",&format!("{prefix}/pause"),&cookie,&csrf,json!({})).await;
    call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":5})).await;
    let id=Uuid::parse_str(prefix.rsplit('/').next().unwrap()).unwrap();
    state.games.write().await.get_mut(&id).unwrap().scheduler.advanced(std::time::Instant::now(),0.25);
    call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":5})).await;
    assert!(!state.games.read().await[&id].scheduler.due(std::time::Instant::now()));
    for session in state.sessions.entries.lock().unwrap().values_mut().filter(|session|session.player_id==player){session.expires=std::time::Instant::now()-Duration::from_secs(1);}
    let task=tokio::spawn(run_simulation_loop(state.clone()));
    tokio::time::sleep(Duration::from_millis(1150)).await;
    task.abort();let _=task.await;
    let games=state.games.read().await;assert!(games[&id].roles[&Uuid::parse_str(&role).unwrap()].owner.is_none());assert_eq!(games[&id].simulation.tick(),0);
}

#[tokio::test]
async fn sixty_times_clock_advances_faster_than_the_old_polling_limit() {
    let fixture=fixture().await;let state=&fixture.state;
    let(cookie,csrf,_)=new_guest(state).await;
    let(prefix,role)=create_game(state,&cookie,&csrf).await;
    claim_seat(state,&prefix,&role,&cookie,&csrf).await;
    assert_eq!(call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":60})).await.0,StatusCode::OK);
    assert_eq!(call(state,"POST",&format!("{prefix}/start"),&cookie,&csrf,json!({})).await.0,StatusCode::OK);
    let id=Uuid::parse_str(prefix.rsplit('/').next().unwrap()).unwrap();
    let task=tokio::spawn(run_simulation_loop(state.clone()));
    tokio::time::sleep(Duration::from_millis(600)).await;
    task.abort();let _=task.await;
    let tick=state.games.read().await[&id].simulation.tick();
    assert!(tick>20,"60x must exceed the old 20-tick/s polling ceiling; observed {tick} ticks in 600 ms");
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL and PostgreSQL CREATEDB permission"]
async fn durable_fractional_frames_replay_before_the_next_whole_game_second() {
    let database=Database::new().await;
    let mut fixture=durable_fixture(&database).await;let state=&mut fixture.state;
    let(cookie,csrf,_)=new_guest(state).await;let(prefix,role)=create_game(state,&cookie,&csrf).await;
    claim_seat(state,&prefix,&role,&cookie,&csrf).await;
    assert_eq!(call(state,"PUT",&format!("{prefix}/time-scale"),&cookie,&csrf,json!({"time_scale":0.25})).await.0,StatusCode::OK);
    assert_eq!(call(state,"POST",&format!("{prefix}/start"),&cookie,&csrf,json!({})).await.0,StatusCode::OK);
    let id=Uuid::parse_str(prefix.rsplit('/').next().unwrap()).unwrap();
    for _ in 0..45 {
        let mut games=state.games.write().await;let game=games.get_mut(&id).unwrap();
        let mut input=recovery::TickInput::capture(game);input.frame_scale=Some(0.25);
        ai_orders::process_ai_orders(game);advance_game_frame(game,0.25);intents::record_execution_results(game);
        let change=recovery::Boundary::new(game,Some(input),None,"simulation_frame").unwrap();
        state.durability.store.as_ref().unwrap().commit(&[change],&mut games,None).await.unwrap();
    }
    let before=serde_json::to_value(state.games.read().await[&id].simulation.snapshot()).unwrap();
    restart(state,&database).await.unwrap();
    {
        let games=state.games.read().await;let game=&games[&id];
        assert_eq!(game.status,GameStatus::Paused);assert_eq!(game.time_scale,Some(0.25));
        assert_eq!(game.simulation.tick(),0);assert_eq!(game.simulation.frame_number(),Some(45));
        assert_eq!(game.simulation.elapsed_game_seconds(),0.1875);
        assert_eq!(serde_json::to_value(game.simulation.snapshot()).unwrap(),before);
    }
    drop(fixture);database.close().await;
}

#[tokio::test]
async fn normal_clock_uses_subsecond_frames_at_one_times_speed() {
    let fixture=fixture().await;let state=&fixture.state;
    let(cookie,csrf,_)=new_guest(state).await;let(prefix,role)=create_game(state,&cookie,&csrf).await;
    claim_seat(state,&prefix,&role,&cookie,&csrf).await;
    assert_eq!(call(state,"POST",&format!("{prefix}/start"),&cookie,&csrf,json!({})).await.0,StatusCode::OK);
    let id=Uuid::parse_str(prefix.rsplit('/').next().unwrap()).unwrap();
    let task=tokio::spawn(run_simulation_loop(state.clone()));
    tokio::time::sleep(Duration::from_millis(600)).await;
    task.abort();let _=task.await;
    let games=state.games.read().await;let sim=&games[&id].simulation;
    let frames=sim.frame_number().unwrap();
    assert!(frames>20,"normal speed must use subsecond frames; observed {frames} in 600 ms");
    assert!((sim.elapsed_game_seconds()-frames as f64/60.0).abs()<1e-12);
}
