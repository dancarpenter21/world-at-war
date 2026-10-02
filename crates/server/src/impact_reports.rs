//! Bounded delivery of firing-terminal telemetry over the same recorded radio lifecycle.
use super::*;
use sim_core::combat::ImpactReport;
use sim_scenario::ImpactReportRoute;

pub(super) const IMPACT_REPORT_PROFILE: &str = "public-safe.jseries.impact-report.v1";
const MAX_ATTEMPTS: u8 = 4;

pub(super) struct ImpactReportState {
    routes: Vec<ImpactReportRoute>,
    attempts: BTreeMap<(Uuid, Uuid, Uuid), (u64, u8)>,
    pub(super) window_ticks: u64,
    pub(super) remaining_ticks: u64,
}
impl ImpactReportState {
    pub(super) fn new(routes: Vec<ImpactReportRoute>, window_ticks: u64) -> Self {
        Self {
            routes,
            attempts: BTreeMap::new(),
            window_ticks,
            remaining_ticks: 0,
        }
    }
}

fn delivered(game: &Game, route: &ImpactReportRoute, intent: Uuid) -> bool {
    game.network_messages.iter().any(|record| {
        record.state == MessageState::Delivered
            && record.message.profile_id == IMPACT_REPORT_PROFILE
            && record.message.header.origin_role_id == route.origin_role_id
            && record.message.header.recipient_entity_id == route.recipient_unit_id
            && record
                .message
                .fields
                .get("intent_id")
                .is_some_and(|id| id.as_str() == Some(intent.to_string().as_str()))
    })
}
fn pending(game: &Game, route: &ImpactReportRoute, intent: Uuid) -> bool {
    game.pending_deliveries.values().any(|action| {
        matches!(action,
        DeliveryAction::ReceiveImpactReport { recipient_unit_id, report }
            if *recipient_unit_id == route.recipient_unit_id && report.intent_id == intent)
    })
}

pub(super) fn reports_settled(game: &Game) -> bool {
    game.impact_reports.routes.iter().all(|route| {
        let Some(role) = game.roles.get(&route.origin_role_id) else {
            return true;
        };
        game.simulation
            .local_impact_reports(role.location_unit_id)
            .iter()
            .all(|report| {
                delivered(game, route, report.intent_id)
                    || (!pending(game, route, report.intent_id)
                        && game
                            .impact_reports
                            .attempts
                            .get(&(
                                route.origin_role_id,
                                route.recipient_unit_id,
                                report.intent_id,
                            ))
                            .is_some_and(|(_, count)| *count >= MAX_ATTEMPTS))
            })
    })
}

pub(super) fn send_reports(game: &mut Game) {
    if game.status != GameStatus::Running || game.impact_reports.routes.is_empty() {
        return;
    }
    if !game.message_profiles.contains_key(IMPACT_REPORT_PROFILE) {
        game.status = GameStatus::Paused;
        game.operational_error =
            Some("impact-report message profile is missing; game paused".into());
        return;
    }
    let tick = game.simulation.radio_tick();
    for route in game.impact_reports.routes.clone() {
        let Some(role) = game.roles.get(&route.origin_role_id) else {
            continue;
        };
        let reports: Vec<ImpactReport> =
            game.simulation.local_impact_reports(role.location_unit_id);
        for report in reports {
            if delivered(game, &route, report.intent_id) || pending(game, &route, report.intent_id)
            {
                continue;
            }
            let key = (
                route.origin_role_id,
                route.recipient_unit_id,
                report.intent_id,
            );
            let attempt = game.impact_reports.attempts.entry(key).or_default();
            if attempt.1 >= MAX_ATTEMPTS
                || (attempt.1 > 0 && tick.saturating_sub(attempt.0) < route.retry_interval_ticks)
            {
                continue;
            }
            *attempt = (tick, attempt.1 + 1);
            let fields = BTreeMap::from([
                ("intent_id".into(), serde_json::json!(report.intent_id)),
                ("impact".into(), serde_json::json!(report)),
            ]);
            if let Some(message_id) = transmit_c2_message_with_fields(
                game,
                route.origin_role_id,
                route.recipient_unit_id,
                IMPACT_REPORT_PROFILE,
                format!(
                    "Training impact: {} at tick {}",
                    if report.hit { "hit" } else { "miss" },
                    report.resolved_tick
                ),
                fields,
            ) {
                await_message_delivery(
                    game,
                    message_id,
                    DeliveryAction::ReceiveImpactReport {
                        recipient_unit_id: route.recipient_unit_id,
                        report,
                    },
                );
            }
            if game.status != GameStatus::Running {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
