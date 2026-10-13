//! Steady wall-clock frame pacing; speed scales fractional game time.
use super::*;
use std::time::Instant;

pub const SCALES: [f64; 7] = [0.25, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0];

#[derive(Default)]
pub(super) struct Scheduler {
    next: Option<Instant>,
}
impl Scheduler {
    pub fn reset(&mut self) {
        self.next = None;
    }
    pub fn due(&self, now: Instant) -> bool {
        self.next.is_none_or(|next| now >= next)
    }
    pub fn delay(&self, now: Instant) -> Duration {
        self.next
            .map_or(Duration::ZERO, |next| next.saturating_duration_since(now))
    }
    pub fn advanced_frame(&mut self, now: Instant) {
        let period = Duration::from_secs_f64(1.0 / f64::from(sim_core::TICKS_PER_SECOND));
        let next = self.next.map_or(now + period, |deadline| deadline + period);
        self.next = Some(if next > now { next } else { now + period });
    }
    #[cfg(test)]
    pub fn advanced(&mut self, now: Instant, scale: f64) {
        // Never build catch-up debt after a slow durable commit.
        self.next = Some(now + Duration::from_secs_f64(1.0 / scale));
    }
}

#[derive(Deserialize)]
pub(super) struct Request {
    pub time_scale: f64,
    pub expected_control_revision: Option<u64>,
}

pub(super) async fn set_scale(
    session: auth::Session,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    AuthJson(request): AuthJson<Request>,
) -> ApiResult<GameSummary> {
    if !SCALES.contains(&request.time_scale) {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_time_scale",
            "Choose 0.25, 1, 2, 5, 10, 30 or 60",
        ));
    }
    let mut games = state.games.write().await;
    let game = games
        .get_mut(&id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "Game not found"))?;
    teams::control(game, session.player_id, request.expected_control_revision)?;
    if game.time_scale.unwrap_or(1.0) != request.time_scale {
        teams::record_control(
            game,
            session.player_id,
            format!("speed {}×", request.time_scale),
        );
        game.scheduler.reset();
        game.diagnostics.reset_rate();
    }
    game.time_scale = Some(request.time_scale);
    Ok(Json(game_summary(game)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rates_have_independent_deadlines_and_do_not_accumulate_catchup() {
        let start = Instant::now();
        for scale in SCALES {
            let mut schedule = Scheduler::default();
            assert!(schedule.due(start));
            schedule.advanced(start, scale);
            let period = Duration::from_secs_f64(1.0 / scale);
            assert_eq!(schedule.delay(start), period);
            assert_eq!(schedule.delay(start + period), Duration::ZERO);
            assert!(!schedule.due(start + period / 2));
            assert!(schedule.due(start + period));
            let late = start + Duration::from_secs(30);
            schedule.advanced(late, scale);
            assert!(!schedule.due(late));
            assert!(schedule.due(late + period));
            schedule.reset();
            assert!(schedule.due(late));
        }
    }
    #[test]
    fn frame_deadlines_preserve_phase_without_catchup_debt() {
        let start = Instant::now();
        let period = Duration::from_secs_f64(1.0 / 60.0);
        let mut schedule = Scheduler::default();
        schedule.advanced_frame(start);
        schedule.advanced_frame(start + period + Duration::from_millis(1));
        assert!(schedule.due(start + period * 2));
        assert!(!schedule.due(start + period * 2 - Duration::from_millis(1)));
        let late = start + Duration::from_secs(2);
        schedule.advanced_frame(late);
        assert_eq!(schedule.delay(late), period);
    }
    #[test]
    fn maintenance_journal_does_not_advance_a_running_game() {
        let mut game = authority_tests::game();
        let tick = game.simulation.tick();
        let mut input = recovery::TickInput::capture(&game);
        input.advance = Some(false);
        input.apply(&mut game).unwrap();
        assert_eq!(game.simulation.tick(), tick);
        let input = recovery::TickInput::capture(&game);
        input.apply(&mut game).unwrap();
        assert_eq!(game.simulation.tick(), tick + 1);
    }
    #[test]
    fn fractional_journal_replays_subsecond_time_and_preserves_legacy_inputs() {
        let mut live = authority_tests::game();
        let mut replay = recovery::decode(&recovery::encode(&live).unwrap()).unwrap();
        for _ in 0..30 {
            let mut input = recovery::TickInput::capture(&live);
            input.frame_scale = Some(1.0);
            ai_orders::process_ai_orders(&mut live);
            advance_game_frame(&mut live, 1.0);
            intents::record_execution_results(&mut live);
            input.apply(&mut replay).unwrap();
            assert_eq!(
                recovery::encode(&live).unwrap(),
                recovery::encode(&replay).unwrap()
            );
        }
        assert_eq!(live.simulation.tick(), 0);
        assert_eq!(live.simulation.elapsed_game_seconds(), 0.5);
    }
    #[test]
    fn selected_speed_survives_snapshot_without_scheduler_deadlines() {
        let mut game = authority_tests::game();
        game.time_scale = Some(5.0);
        game.scheduler.advanced(Instant::now(), 5.0);
        let bytes = recovery::encode(&game).unwrap();
        let restored = recovery::decode(&bytes).unwrap();
        assert_eq!(restored.time_scale, Some(5.0));
        assert!(restored.scheduler.due(Instant::now()));
    }
}
