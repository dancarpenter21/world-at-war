// Included in auth::tests to reuse the real cookie/CSRF/router fixture.
#[tokio::test]
async fn observer_grants_require_host_and_truth_requires_a_current_granted_lease() {
    let fixture = fixture().await;
    let state = &fixture.state;
    let (host, host_csrf, host_id) = new_guest(state).await;
    let (guest, guest_csrf, guest_id) = new_guest(state).await;
    let (_, _, created) = call(
        state,
        "POST",
        "/v1/games",
        &host,
        &host_csrf,
        json!({"scenario_id":"regional-campaign.v1"}),
    )
    .await;
    let prefix = format!("/v1/games/{}", created["game"]["id"].as_str().unwrap());
    let seat = Uuid::from_u128(9001);
    let grant = format!("{prefix}/roles/{seat}/observer-grant");
    let claim = format!("{prefix}/roles/{seat}/claim");
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/truth?role_id={seat}&lease_generation=0"),
            &host,
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
            &grant,
            &host,
            &host_csrf,
            json!({"target_player_id":guest_id})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/join"),
            &guest,
            &guest_csrf,
            json!({"display_name":"Monitor guest"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/participants"),
            &guest,
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
            &grant,
            &guest,
            &guest_csrf,
            json!({"target_player_id":guest_id})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            state,
            "PUT",
            &grant,
            &host,
            "wrong-csrf",
            json!({"target_player_id":guest_id})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            state,
            "PUT",
            &grant,
            &host,
            &host_csrf,
            json!({"target_player_id":guest_id,"player_id":host_id})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(state, "POST", &claim, &guest, &guest_csrf, json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
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
    let (status, _, lease) = call(state, "POST", &claim, &guest, &guest_csrf, json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let generation = lease["lease_generation"].as_u64().unwrap();
    assert_eq!(lease["observer_kind"], "controller");
    assert!(lease.get("location_unit_id").is_none());
    let query = format!("role_id={seat}&lease_generation={generation}");
    let (status, headers, truth) = call(
        state,
        "GET",
        &format!("{prefix}/truth?{query}"),
        &guest,
        "",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[axum::http::header::CACHE_CONTROL], "no-store");
    assert!(truth["units"]
        .as_array()
        .unwrap()
        .iter()
        .any(|u| u["side"] == "Red"));
    assert!(truth.get("messages").is_none());
    assert!(truth.get("tracks").is_none());
    for endpoint in [
        "state",
        "network",
        "network/events",
        "planning",
        "diagnostics",
    ] {
        let status = call(
            state,
            "GET",
            &format!("{prefix}/{endpoint}?{query}"),
            &guest,
            "",
            Value::Null,
        )
        .await
        .0;
        assert!(
            !status.is_success(),
            "observer unexpectedly read {endpoint}"
        );
    }
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/pause"),
            &guest,
            &guest_csrf,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for kind in [
        json!({"Move":{"north_mps":10.0,"east_mps":0.0}}),
        json!({"Engage":{"track_id":Uuid::from_u128(700)}}),
        json!({"Iftu":{"weapon_id":Uuid::from_u128(701),"command":{"action":"assign_provider","provider_id":Uuid::from_u128(11)}}}),
    ] {
        let (status, _, error) = call(state, "POST", &format!("{prefix}/roles/{seat}/intent"), &guest, &guest_csrf,
            json!({"lease_generation":generation,"intent":{"intent_id":Uuid::new_v4(),"issuer_role":seat,"target":Uuid::from_u128(11),"kind":kind,"requested_tick":1}})).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["code"], "role_not_found");
    }
    for (path, body) in [
        (
            format!("{prefix}/roles/{seat}/planning"),
            json!({"lease_generation":generation,"action":"propose"}),
        ),
        (
            format!("{prefix}/roles/{seat}/authority-requests"),
            json!({"lease_generation":generation,"action":"move","target_unit_id":Uuid::from_u128(11),"summary":"Observer command"}),
        ),
        (
            format!(
                "{prefix}/roles/{seat}/authority-requests/{}/decision",
                Uuid::new_v4()
            ),
            json!({"lease_generation":generation,"decision":"approve"}),
        ),
    ] {
        assert_eq!(
            call(state, "POST", &path, &guest, &guest_csrf, body)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }
    let (_, _, participants) = call(
        state,
        "GET",
        &format!("{prefix}/participants"),
        &host,
        "",
        Value::Null,
    )
    .await;
    assert!(participants.as_array().unwrap().iter().any(
        |p| p["display_name"] == "Monitor guest" && p["observer_seats"][0] == seat.to_string()
    ));
    let (_, _, definition) = call(
        state,
        "GET",
        &format!("{prefix}/authority"),
        &host,
        "",
        Value::Null,
    )
    .await;
    let mut malicious = definition.clone();
    malicious["roles"][0]["id"] = json!(seat);
    // Either normal graph validation or the explicit observer-ID check must reject it.
    assert_eq!(
        call(
            state,
            "PUT",
            &format!("{prefix}/authority"),
            &host,
            &host_csrf,
            json!({"expected_version":definition["version"],"definition":malicious})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let mut valid = definition.clone();
    valid["roles"][0]["name"] = json!("Renamed commander");
    assert_eq!(
        call(
            state,
            "PUT",
            &format!("{prefix}/authority"),
            &host,
            &host_csrf,
            json!({"expected_version":definition["version"],"definition":valid})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/truth?{query}"),
            &guest,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, _, second) = call(
        state,
        "POST",
        "/v1/games",
        &host,
        &host_csrf,
        json!({"scenario_id":"regional-campaign.v1"}),
    )
    .await;
    assert_eq!(
        call(
            state,
            "GET",
            &format!(
                "/v1/games/{}/truth?{query}",
                second["game"]["id"].as_str().unwrap()
            ),
            &guest,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    // Replacement while paused invalidates the old generation and guest immediately.
    assert_eq!(
        call(
            state,
            "POST",
            &format!("{prefix}/pause"),
            &host,
            &host_csrf,
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            state,
            "PUT",
            &grant,
            &host,
            &host_csrf,
            json!({"target_player_id":host_id})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/truth?{query}"),
            &guest,
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
            "POST",
            &format!("{prefix}/roles/{seat}/resume"),
            &guest,
            &guest_csrf,
            json!({})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, _, replacement) = call(state, "POST", &claim, &host, &host_csrf, json!({})).await;
    assert!(replacement["lease_generation"].as_u64().unwrap() > generation);
    assert_eq!(
        call(
            state,
            "GET",
            &format!("{prefix}/truth?{query}"),
            &host,
            "",
            Value::Null
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(state, "DELETE", &grant, &host, &host_csrf, json!({}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(state, "POST", &claim, &host, &host_csrf, json!({}))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}
