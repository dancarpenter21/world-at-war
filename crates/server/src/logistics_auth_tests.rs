#[tokio::test]
async fn logistics_orders_use_held_authority_and_delivered_execution_with_idempotent_receipts() {
    let mut fixture=fixture().await;
    let scenario=sim_scenario::joint_sustainment_exercise_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(),scenario);
    let state=&fixture.state;let host=new_guest(state).await;let stranger=new_guest(state).await;
    let created=team_call(state,&host,"POST","/v1/games",json!({"scenario_id":"joint-sustainment-exercise.v1"})).await;
    let game_id=Uuid::parse_str(created["game"]["id"].as_str().unwrap()).unwrap();let prefix=format!("/v1/games/{game_id}");
    let role=Uuid::from_u128(20102);let held=claim_seat(state,&prefix,&role.to_string(),&host.0,&host.1).await;
    team_call(state,&host,"POST",&format!("{prefix}/start"),json!({})).await;
    let intent=sim_core::PlayerIntent{intent_id:Uuid::from_u128(123456),issuer_role:role,target:Uuid::from_u128(13),kind:sim_core::OrderKind::Logistics{command:sim_core::logistics::Command::Support{interface:sim_core::logistics::Interface::Boom}},requested_tick:1};
    let body=json!({"lease_generation":held["lease_generation"],"intent":intent});
    let url=format!("{prefix}/roles/{role}/intent");
    assert_eq!(call(state,"POST",&url,&stranger.0,&stranger.1,body.clone()).await.0,StatusCode::FORBIDDEN);
    let (status,_,submission)=call(state,"POST",&url,&host.0,&host.1,body.clone()).await;assert_eq!(status,StatusCode::OK,"{submission}");
    assert_eq!(state.games.read().await[&game_id].simulation.tick(),0);
    let (status,_,duplicate)=call(state,"POST",&url,&host.0,&host.1,body).await;assert_eq!(status,StatusCode::OK);assert_eq!(submission,duplicate);
    let receipt_url=format!("{prefix}/roles/{role}/intents/{}?lease_generation={}",intent.intent_id,held["lease_generation"]);
    assert!(team_call(state,&host,"GET",&receipt_url,Value::Null).await.get("executed_tick").is_none());
    {
        let mut games=state.games.write().await;let game=games.get_mut(&game_id).unwrap();
        assert_eq!(game.simulation.logistics_projection(Uuid::from_u128(5)).unwrap().units.len(),0);
        for _ in 0..30 {advance_game_tick(game);}
    }
    let receipt=team_call(state,&host,"GET",&receipt_url,Value::Null).await;assert_eq!(receipt["state"],"executed","{receipt}");
    let picture=team_call(state,&host,"GET",&format!("{prefix}/state?role_id={role}&lease_generation={}",held["lease_generation"]),Value::Null).await;
    assert_eq!(picture["logistics"]["units"].as_array().unwrap().len(),6);
    assert!(picture["logistics"]["units"].as_array().unwrap().iter().all(|u|u["received_tick"].as_u64().unwrap()>u["observed_tick"].as_u64().unwrap()));
}

#[tokio::test]
async fn logistics_reports_cannot_be_forged_from_another_origin_role() {
    let mut fixture=fixture().await;let scenario=sim_scenario::joint_sustainment_exercise_scenario();
    Arc::make_mut(&mut fixture.state.scenarios).insert(scenario.id.clone(),scenario);
    let state=&fixture.state;let host=new_guest(state).await;
    let created=team_call(state,&host,"POST","/v1/games",json!({"scenario_id":"joint-sustainment-exercise.v1"})).await;
    let id=Uuid::parse_str(created["game"]["id"].as_str().unwrap()).unwrap();
    let mut games=state.games.write().await;let game=games.get_mut(&id).unwrap();
    let report=game.simulation.local_logistics_report(Uuid::from_u128(11)).unwrap();
    let message=sim_comms::C2Message {id:Uuid::from_u128(999999),profile_id:"simulation.joint-operations.v1".into(),header:sim_comms::MessageHeader {origin_role_id:Uuid::from_u128(20102),origin_entity_id:Uuid::from_u128(5),recipient_entity_id:Uuid::from_u128(5),classification:"simulation-controlled".into(),priority:0,created_tick:0,expires_tick:60},fields:std::collections::BTreeMap::from([("operation".into(),json!({"kind":"logistics_report","recipient":Uuid::from_u128(5),"source":Uuid::from_u128(11),"report":report}))]),rendered_text:"forged".into()};
    planning::receive(game,&serde_json::to_vec(&message).unwrap());
    assert_eq!(game.simulation.logistics_projection(Uuid::from_u128(5)).unwrap().units.len(),0);
}
