use super::*;
fn runtime() -> Runtime {
    Runtime::new(
        Configuration {
            radio_upgrade: None,
            model: Model::training(),
            terrain_required: false,
            terrain_fixture: None,
            sensors: BTreeMap::new(),
            signatures: BTreeMap::new(),
            weather: Weather::default(),
            jammers: vec![],
        },
        42,
        Uuid::from_u128(1),
    )
    .unwrap()
}
fn track(id: u128, tick: u64, lon: f64) -> Track {
    Track {
        track_id: Uuid::from_u128(id),
        target_side: None,
        position: GeoPose {
            latitude_deg: 38.0,
            longitude_deg: lon,
            altitude_m: 1000.0,
        },
        identity_confidence: 0.45,
        observed_tick: tick,
        received_tick: tick,
        observed_sidc: "100101000000000000000000000000".into(),
        uncertainty_m: 245.0,
        assessed_destroyed: None,
        observed_domain: None,
        evidence: None,
    }
}
#[test]
fn fuses_measurements_without_truth_ids_and_deduplicates_reports() {
    let mut r = runtime();
    let owner = Uuid::from_u128(2);
    assert!(r.receive(owner, track(10, 1, -76.0), 1, true));
    assert!(r.receive(owner, track(11, 2, -76.0001), 2, true));
    assert_eq!(r.tracks_for(owner, 2).len(), 1);
    let report = r.tracks_for(owner, 2)[0].clone();
    let target = Uuid::from_u128(3);
    assert!(r.receive(target, report.clone(), 5, false));
    assert!(!r.receive(target, report, 6, false));
    assert_eq!(r.tracks_for(target, 6)[0].received_tick, 5);
}
#[test]
fn broad_unrelated_estimate_does_not_make_precise_updates_ambiguous() {
    let mut r = runtime();
    let owner = Uuid::from_u128(2);
    assert!(r.receive(owner, track(100, 1, 0.0), 1, true));
    assert!(r.receive(owner, track(101, 1, 0.1), 1, true));
    let broad = &mut r.tracks.get_mut(&owner).unwrap()[1];
    broad.covariance = diagonal(1_000_000.0);
    broad.track.evidence.as_mut().unwrap().covariance = broad.covariance;
    assert!(r.receive(owner, track(102, 2, 0.00001), 2, true));
    assert_eq!(r.tracks_for(owner, 2).len(), 2);
}

#[test]
fn uncertainty_grows_and_obsolete_reports_do_not_rejuvenate_tracks() {
    let mut r = runtime();
    let owner = Uuid::from_u128(2);
    r.receive(owner, track(10, 1, -76.0), 1, true);
    let fresh = r.tracks_for(owner, 1)[0].uncertainty_m;
    assert!(r.tracks_for(owner, 30)[0].uncertainty_m > fresh);
    assert!(r.tracks_for(owner, 122).is_empty());
    assert!(!r.receive(owner, track(11, 1, -76.0), 122, false));
}
#[test]
fn bearings_require_distinct_geometry_and_never_expose_an_entity_id() {
    let mut r = runtime();
    let owner = Uuid::from_u128(2);
    let a = Bearing {
        maximum_range_m: 180000.0,
        source: Uuid::from_u128(20),
        id: Uuid::from_u128(10),
        receiver_position: GeoPose {
            latitude_deg: 38.0,
            longitude_deg: -76.0,
            altitude_m: 0.0,
        },
        bearing_deg: 45.0,
        uncertainty_deg: 2.0,
        observed_tick: 1,
        received_tick: 1,
        emitter_class: "fictional-search".into(),
    };
    assert!(r.receive_bearing(owner, a.clone(), 1));
    assert!(r.tracks_for(owner, 1).is_empty());
    let mut b = a.clone();
    b.id = Uuid::from_u128(11);
    b.source = Uuid::from_u128(21);
    b.receiver_position.longitude_deg = -75.9;
    b.bearing_deg = 315.0;
    assert!(r.receive_bearing(owner, b, 1));
    assert_eq!(r.tracks_for(owner, 1).len(), 1);
    let mut b = a.clone();
    b.id = Uuid::from_u128(12);
    b.receiver_position.longitude_deg = -75.9;
    assert!(triangulate(&a, &b).is_none());
}
#[test]
fn multiple_same_class_emitters_remain_bearings_instead_of_arbitrary_fixes() {
    let mut r = runtime();
    let owner = Uuid::from_u128(2);
    let a = Bearing {
        maximum_range_m: 180000.0,
        source: Uuid::from_u128(20),
        id: Uuid::from_u128(10),
        receiver_position: GeoPose {
            latitude_deg: 38.0,
            longitude_deg: -76.0,
            altitude_m: 0.0,
        },
        bearing_deg: 45.0,
        uncertainty_deg: 2.0,
        observed_tick: 1,
        received_tick: 1,
        emitter_class: "fictional-search".into(),
    };
    r.receive_bearing(owner, a.clone(), 1);
    let mut ambiguous = a.clone();
    ambiguous.id = Uuid::from_u128(12);
    ambiguous.bearing_deg = 90.0;
    r.receive_bearing(owner, ambiguous, 1);
    let mut other = a;
    other.id = Uuid::from_u128(11);
    other.source = Uuid::from_u128(21);
    other.receiver_position.longitude_deg = -75.9;
    other.bearing_deg = 315.0;
    r.receive_bearing(owner, other, 1);
    assert!(r.tracks_for(owner, 1).is_empty());
    assert_eq!(r.bearings[&owner].len(), 3);
}

#[test]
fn environment_updates_are_validated_revisioned_and_tick_bound() {
    let mut r = runtime();
    let baseline = Weather {
        visibility_m: 100.0,
        ..Weather::default()
    };
    r.environment
        .schedule(0, 5, baseline.clone(), vec![])
        .unwrap();
    assert_eq!(r.environment.baseline.visibility_m, 100000.0);
    assert!(r.environment.schedule(0, 6, baseline, vec![]).is_err());
    r.environment.advance(4);
    assert!(r.environment.pending.is_some());
    r.environment.advance(5);
    assert_eq!(r.environment.baseline.visibility_m, 100.0);
    assert!(r.environment.pending.is_none());
}
#[test]
fn radar_rain_clutter_and_jamming_reduce_detection_and_close_range_burns_through() {
    let model = Model::training();
    let sensor = SensorSuite {
        range_m: 100000.0,
        identification_range_m: 10000.0,
        scan_interval_ticks: 1,
        field_of_regard_deg: 360.0,
        modalities: vec![Modality::Radar],
    };
    let signature = Signature {
        radar: 1.0,
        contrast: 1.0,
        thermal: 1.0,
        emitter_class: "fictional".into(),
    };
    let a = GeoPose {
        latitude_deg: 38.0,
        longitude_deg: -76.0,
        altitude_m: 8000.0,
    };
    let b = GeoPose {
        altitude_m: 100.0,
        ..a
    };
    let clear = detection_probability(
        Modality::Radar,
        90000.0,
        &sensor,
        &signature,
        &Weather::default(),
        a,
        b,
        Velocity::default(),
        &model,
        &[],
        1,
    );
    let rain = detection_probability(
        Modality::Radar,
        90000.0,
        &sensor,
        &signature,
        &Weather {
            precipitation: 1.0,
            ..Weather::default()
        },
        a,
        b,
        Velocity::default(),
        &model,
        &[],
        1,
    );
    assert!(rain < clear);
    let close = detection_probability(
        Modality::Radar,
        1000.0,
        &sensor,
        &signature,
        &Weather::default(),
        a,
        b,
        Velocity::default(),
        &model,
        &[],
        1,
    );
    assert_eq!(close, 1.0);
}

#[test]
fn reports_do_not_predict_twice_or_accept_older_source_updates() {
    let mut r = runtime();
    let owner = Uuid::from_u128(2);
    let recipient = Uuid::from_u128(3);
    r.receive(owner, track(10, 1, -76.0), 1, true);
    r.receive(owner, track(11, 2, -75.999), 2, true);
    let report = r.tracks_for(owner, 10)[0].clone();
    let expected = report.position;
    assert_eq!(report.evidence.as_ref().unwrap().estimate_tick, 10);
    assert!(r.receive(recipient, report.clone(), 10, false));
    let received = r.tracks_for(recipient, 10)[0].position;
    assert!(geodesy::surface_distance_m(expected, received) < 0.01);
    let mut older = report;
    older.observed_tick = 1;
    older
        .evidence
        .as_mut()
        .unwrap()
        .observations
        .insert(Uuid::from_u128(99));
    assert!(!r.receive(recipient, older, 11, false));
}

#[test]
fn observation_noise_and_scoped_ids_ignore_unobserved_enemy_truth() {
    let mut r = runtime();
    let observer = Uuid::from_u128(2);
    r.configuration.sensors.insert(
        observer,
        SensorSuite {
            range_m: 20000.0,
            identification_range_m: 10000.0,
            scan_interval_ticks: 1,
            field_of_regard_deg: 360.0,
            modalities: vec![Modality::Visual],
        },
    );
    let target = Uuid::from_u128(3);
    r.configuration.signatures.insert(
        target,
        Signature {
            radar: 1.0,
            contrast: 10.0,
            thermal: 1.0,
            emitter_class: "fictional".into(),
        },
    );
    let a = Platform {
        id: observer,
        side: Side::Blue,
        domain: Domain::Air,
        pose: GeoPose {
            latitude_deg: 0.0,
            longitude_deg: 0.0,
            altitude_m: 1000.0,
        },
        velocity: Velocity::default(),
        destroyed: false,
    };
    let b = Platform {
        id: target,
        side: Side::Red,
        pose: GeoPose {
            longitude_deg: 0.01,
            ..a.pose
        },
        ..a.clone()
    };
    let hidden = Platform {
        id: Uuid::from_u128(4),
        side: Side::Red,
        pose: GeoPose {
            latitude_deg: 70.0,
            ..a.pose
        },
        ..a.clone()
    };
    let mut other = r.clone();
    r.observe(1, &[a.clone(), b.clone()], &[]);
    other.observe(1, &[a, b, hidden], &[]);
    assert_eq!(
        serde_json::to_vec(&r.tracks_for(observer, 1)).unwrap(),
        serde_json::to_vec(&other.tracks_for(observer, 1)).unwrap()
    );
    let visible = &r.tracks_for(observer, 1)[0];
    assert!(visible.position.longitude_deg != 0.01);
    assert!(!serde_json::to_string(visible)
        .unwrap()
        .contains(&target.to_string()));
}

#[test]
fn cloudy_optical_sensors_and_nightlight_reduce_detection() {
    let m = Model::training();
    let sensor = SensorSuite {
        range_m: 10000.0,
        identification_range_m: 5000.0,
        scan_interval_ticks: 1,
        field_of_regard_deg: 360.0,
        modalities: vec![Modality::Visual],
    };
    let signature = Signature {
        radar: 1.0,
        contrast: 1.0,
        thermal: 1.0,
        emitter_class: "fictional".into(),
    };
    let source = GeoPose {
        latitude_deg: 0.0,
        longitude_deg: 0.0,
        altitude_m: 8000.0,
    };
    let target = GeoPose {
        altitude_m: 0.0,
        ..source
    };
    let clear = detection_probability(
        Modality::Visual,
        8000.0,
        &sensor,
        &signature,
        &Weather::default(),
        source,
        target,
        Velocity::default(),
        &m,
        &[],
        1,
    );
    let cloudy = detection_probability(
        Modality::Visual,
        8000.0,
        &sensor,
        &signature,
        &Weather {
            cloud_opacity: 1.0,
            ..Weather::default()
        },
        source,
        target,
        Velocity::default(),
        &m,
        &[],
        1,
    );
    assert!(cloudy < clear);
    let night = detection_probability(
        Modality::Visual,
        8000.0,
        &sensor,
        &signature,
        &Weather::default(),
        GeoPose {
            longitude_deg: 180.0,
            ..source
        },
        target,
        Velocity::default(),
        &m,
        &[],
        1,
    );
    assert!(night < clear);
}
