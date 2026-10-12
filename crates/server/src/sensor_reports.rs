//! Explicit, paced reports from local sensors over the existing packet engine.
use super::*;
use sim_scenario::SensorReportRoute;

pub(super) const TRACK_REPORT_PROFILE: &str = "public-safe.jseries.track-report.v1";

#[derive(Default, Serialize, Deserialize)]
pub(super) struct SensorReportState {
    routes: Vec<SensorReportRoute>,
    #[serde(with = "sim_core::snapshot::entries")]
    last_attempt: BTreeMap<(Uuid, Uuid, Uuid), u64>,
}

impl SensorReportState {
    pub(super) fn upgrade_pacing(&mut self, minimum_ticks: u64) {
        for route in &mut self.routes {
            route.interval_ticks = route.interval_ticks.max(minimum_ticks);
        }
    }
    pub(super) fn new(routes: Vec<SensorReportRoute>) -> Self {
        Self {
            routes,
            ..Self::default()
        }
    }
}

pub(super) fn send_current_reports(game: &mut Game) {
    if game.status != GameStatus::Running || game.sensor_reports.routes.is_empty() {
        return;
    }
    if !game.message_profiles.contains_key(TRACK_REPORT_PROFILE) {
        game.status = GameStatus::Paused;
        game.operational_error =
            Some("track-report message profile is missing; game paused".into());
        return;
    }
    let tick = game.simulation.tick();
    if let Some(config) = game.simulation.sensing_configuration() {
        let expiry = config.model.track_expiry_ticks;
        game.sensor_reports
            .last_attempt
            .retain(|_, last| tick.saturating_sub(*last) <= expiry);
    }
    for route in game.sensor_reports.routes.clone() {
        // Keep each subscription's reliable batch bounded while acknowledgements
        // settle. The next batch samples current knowledge, not a queued backlog.
        if game.simulation.sensing_configuration().is_some()
            && game.network_messages.iter().any(|record| {
                record.message.profile_id == TRACK_REPORT_PROFILE
                    && record.message.header.origin_role_id == route.origin_role_id
                    && record.message.header.recipient_entity_id == route.recipient_unit_id
                    && matches!(
                        record.state,
                        MessageState::Queued
                            | MessageState::InTransit
                            | MessageState::Retrying
                            | MessageState::Delivered
                    )
            })
        {
            continue;
        }
        let Some(role) = game.roles.get(&route.origin_role_id) else {
            continue;
        };
        let location_unit_id = role.location_unit_id;
        let side = role.side;
        let tracks = game.simulation.current_sensor_tracks(location_unit_id);
        for track in tracks {
            let key = (
                route.origin_role_id,
                route.recipient_unit_id,
                track.track_id,
            );
            let pending = game.pending_deliveries.values().any(|action| matches!(action,
                DeliveryAction::ReceiveTrackReport { recipient_unit_id, source_track }
                    if *recipient_unit_id == route.recipient_unit_id && source_track.track_id == track.track_id));
            if pending
                || game
                    .sensor_reports
                    .last_attempt
                    .get(&key)
                    .is_some_and(|last| tick.saturating_sub(*last) < route.interval_ticks)
            {
                continue;
            }
            // Failed or dropped reports are paced too. Each retry samples the latest
            // local observation rather than replaying an old measurement as current.
            game.sensor_reports.last_attempt.insert(key, tick);
            let mut fields = BTreeMap::from([(
                "track".into(),
                serde_json::json!({
                    "track_id": track.track_id, "target_side": track.target_side,
                    "position": track.position, "identity_confidence": track.identity_confidence,
                    "observed_tick": track.observed_tick, "received_tick": track.received_tick,
                    "observed_sidc": track.observed_sidc,
                }),
            )]);
            if let Some(evidence) = &track.evidence {
                fields.get_mut("track").unwrap()["uncertainty_m"] =
                    serde_json::json!(track.uncertainty_m);
                fields.get_mut("track").unwrap()["evidence"] = serde_json::json!(evidence);
            }
            let Some(message_id) = transmit_c2_message_with_fields(
                game,
                route.origin_role_id,
                route.recipient_unit_id,
                TRACK_REPORT_PROFILE,
                format!(
                    "Uncertain {:?} contact observed at tick {}",
                    track.target_side, track.observed_tick
                ),
                fields,
            ) else {
                if game.status != GameStatus::Running {
                    return;
                }
                continue;
            };
            await_message_delivery(
                game,
                message_id,
                DeliveryAction::ReceiveTrackReport {
                    recipient_unit_id: route.recipient_unit_id,
                    source_track: track,
                },
            );
        }
        if game.simulation.sensing_configuration().is_none() {
            continue;
        }
        let weather_key = (
            route.origin_role_id,
            route.recipient_unit_id,
            Uuid::new_v5(&route.origin_role_id, b"weather"),
        );
        if game
            .sensor_reports
            .last_attempt
            .get(&weather_key)
            .is_none_or(|last| tick.saturating_sub(*last) >= route.interval_ticks)
        {
            if let Some(report) = game.simulation.local_weather_report(location_unit_id) {
                game.sensor_reports.last_attempt.insert(weather_key, tick);
                if let Some(message_id) = transmit_c2_message_with_fields(
                    game,
                    route.origin_role_id,
                    route.recipient_unit_id,
                    TRACK_REPORT_PROFILE,
                    "Local weather observation".into(),
                    BTreeMap::from([("weather".into(), serde_json::json!(report))]),
                ) {
                    await_message_delivery(
                        game,
                        message_id,
                        DeliveryAction::ReceiveWeatherReport {
                            recipient_unit_id: route.recipient_unit_id,
                            report,
                        },
                    );
                }
            }
        }
        let local = game
            .simulation
            .projection_for(location_unit_id, side)
            .sensing;
        for bearing in local
            .into_iter()
            .flat_map(|s| s.bearings)
            .filter(|b| b.observed_tick == tick)
        {
            let key = (route.origin_role_id, route.recipient_unit_id, bearing.id);
            if game.sensor_reports.last_attempt.contains_key(&key) {
                continue;
            }
            game.sensor_reports.last_attempt.insert(key, tick);
            if let Some(message_id) = transmit_c2_message_with_fields(
                game,
                route.origin_role_id,
                route.recipient_unit_id,
                TRACK_REPORT_PROFILE,
                "Passive bearing observation".into(),
                BTreeMap::from([("bearing".into(), serde_json::json!(bearing))]),
            ) {
                await_message_delivery(
                    game,
                    message_id,
                    DeliveryAction::ReceiveBearingReport {
                        recipient_unit_id: route.recipient_unit_id,
                        bearing,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
