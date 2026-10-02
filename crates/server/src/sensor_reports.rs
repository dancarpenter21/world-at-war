//! Explicit, paced reports from local sensors over the existing packet engine.
use super::*;
use sim_scenario::SensorReportRoute;

pub(super) const TRACK_REPORT_PROFILE: &str = "public-safe.jseries.track-report.v1";

#[derive(Default)]
pub(super) struct SensorReportState {
    routes: Vec<SensorReportRoute>,
    last_attempt: BTreeMap<(Uuid, Uuid, Uuid), u64>,
}

impl SensorReportState {
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
    for route in game.sensor_reports.routes.clone() {
        let Some(role) = game.roles.get(&route.origin_role_id) else {
            continue;
        };
        let tracks = game.simulation.current_sensor_tracks(role.location_unit_id);
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
            let fields = BTreeMap::from([(
                "track".into(),
                serde_json::to_value(&track).expect("track serialization is infallible"),
            )]);
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
    }
}

#[cfg(test)]
mod tests;
