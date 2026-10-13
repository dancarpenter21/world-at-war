//! Bounded observational telemetry; never used to advance simulation or authorize a role.
use super::*;
use std::{collections::VecDeque, time::Instant};

pub(super) const WINDOW: usize = 120;
#[derive(Default)]
pub(super) struct Samples(VecDeque<f64>);
impl Samples {
    pub fn push(&mut self, value: f64) {
        if self.0.len() == WINDOW {
            self.0.pop_front();
        }
        self.0.push_back(value);
    }
    pub fn summary(&self) -> Summary {
        let mut sorted: Vec<_> = self.0.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        Summary {
            samples: sorted.len(),
            last: self.0.back().copied(),
            p95: (!sorted.is_empty()).then(|| sorted[(sorted.len() * 95).div_ceil(100) - 1]),
            max: sorted.last().copied(),
        }
    }
}
#[derive(Debug, Serialize)]
pub(super) struct Summary {
    samples: usize,
    last: Option<f64>,
    p95: Option<f64>,
    max: Option<f64>,
}
#[derive(Default)]
pub(super) struct Diagnostics {
    tick_ms: Samples,
    last_tick: Option<Instant>,
    committed_intervals_ms: Samples,
    committed_seconds: Samples,
    committed_frames: Samples,
    last_commit: Option<Instant>,
    schedule_delay_ms: f64,
    roles: BTreeMap<Uuid, RoleSamples>,
}
#[derive(Default)]
struct RoleSamples {
    build_ms: Samples,
    serialization_ms: Samples,
    tick: u64,
    snapshot_bytes: usize,
    visible_queue_packets: usize,
    visible_queue_bytes: usize,
}
impl Diagnostics {
    pub fn reset_rate(&mut self) {
        self.last_tick = None;
        self.last_commit = None;
        self.committed_intervals_ms = Samples::default();
        self.committed_seconds = Samples::default();
        self.committed_frames = Samples::default();
    }
    pub fn record_tick(&mut self, elapsed_ms: f64, schedule_delay_ms: f64) {
        self.tick_ms.push(elapsed_ms);
        self.last_tick = Some(Instant::now());
        self.schedule_delay_ms = schedule_delay_ms;
    }
    pub fn record_commit(&mut self, now: Instant, frames: u64, game_seconds: f64) {
        if frames == 0 {
            return;
        }
        if let Some(last) = self.last_commit {
            self.committed_intervals_ms
                .push(now.duration_since(last).as_secs_f64() * 1000.0);
            self.committed_seconds.push(game_seconds);
            self.committed_frames.push(frames as f64);
        }
        self.last_commit = Some(now);
    }
    #[cfg(test)]
    pub fn last_committed_sample_absent(&self) -> bool {
        self.last_commit.is_none()
    }
    fn committed_rate(&self, samples: &Samples, now: Instant) -> Option<f64> {
        let last = self.last_commit?;
        if samples.0.is_empty() || now.duration_since(last) > Duration::from_secs(3) {
            return None;
        }
        let elapsed_ms = self.committed_intervals_ms.0.iter().sum::<f64>()
            + now.duration_since(last).as_secs_f64() * 1000.0;
        (elapsed_ms > 0.0).then(|| samples.0.iter().sum::<f64>() * 1000.0 / elapsed_ms)
    }
}

/// Serialize once; use the same bytes for the response and its size measurement.
pub(super) fn projection_bytes(game: &mut Game, role: &Role) -> Result<Vec<u8>, serde_json::Error> {
    let start = Instant::now();
    let projection = game
        .simulation
        .projection_for(role.location_unit_id, role.side);
    let build_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    let bytes = serde_json::to_vec(&projection)?;
    let serialization_ms = start.elapsed().as_secs_f64() * 1000.0;
    let samples = game.diagnostics.roles.entry(role.id).or_default();
    samples.build_ms.push(build_ms);
    samples.serialization_ms.push(serialization_ms);
    samples.tick = projection.tick;
    samples.snapshot_bytes = bytes.len();
    samples.visible_queue_packets = projection
        .communication_links
        .iter()
        .map(|link| link.queued_packets)
        .sum();
    samples.visible_queue_bytes = projection
        .communication_links
        .iter()
        .map(|link| link.queued_bytes)
        .sum();
    performance::record(|| {
        serde_json::json!({"kind": "projection", "transport": "http", "game_id": game.id,
        "role_id": role.id, "build_ms": build_ms, "serialization_ms": serialization_ms, "bytes": bytes.len()})
    });
    Ok(bytes)
}

#[derive(Serialize)]
pub(super) struct View {
    requested_time_scale: f64,
    achieved_time_scale: Option<f64>,
    target_frame_hz: u32,
    committed_frame_hz: Option<f64>,
    committed_frame_interval_ms: Summary,
    committed_frame_overruns: usize,
    last_committed_frame_age_ms: Option<f64>,
    tick: u64,
    status: GameStatus,
    window_capacity: usize,
    server_tick_ms: Summary,
    simulation_ms: sim_core::performance::StepTimings,
    server_tick_overruns: usize,
    last_tick_age_ms: Option<f64>,
    schedule_delay_ms: f64,
    projection: Option<ProjectionView>,
}
#[derive(Serialize)]
struct ProjectionView {
    tick: u64,
    build_ms: Summary,
    serialization_ms: Summary,
    snapshot_bytes: usize,
    visible_queue_packets: usize,
    visible_queue_bytes: usize,
}
fn view(game: &Game, query: &ProjectionQuery) -> Result<View, (StatusCode, Json<ErrorResponse>)> {
    let role = authorized_role(game, query)?;
    let diagnostics = &game.diagnostics;
    Ok(View {
        requested_time_scale: game.time_scale.unwrap_or(1.0),
        achieved_time_scale: (game.status == GameStatus::Running)
            .then(|| diagnostics.committed_rate(&diagnostics.committed_seconds, Instant::now()))
            .flatten(),
        target_frame_hz: sim_core::TICKS_PER_SECOND,
        committed_frame_hz: (game.status == GameStatus::Running)
            .then(|| diagnostics.committed_rate(&diagnostics.committed_frames, Instant::now()))
            .flatten(),
        committed_frame_interval_ms: diagnostics.committed_intervals_ms.summary(),
        committed_frame_overruns: diagnostics
            .committed_intervals_ms
            .0
            .iter()
            .filter(|ms| **ms > 1000.0 / f64::from(sim_core::TICKS_PER_SECOND))
            .count(),
        last_committed_frame_age_ms: diagnostics
            .last_commit
            .map(|at| at.elapsed().as_secs_f64() * 1000.0),
        tick: game.simulation.tick(),
        status: game.status,
        window_capacity: WINDOW,
        server_tick_ms: diagnostics.tick_ms.summary(),
        simulation_ms: game.simulation.last_step_timings(),
        server_tick_overruns: diagnostics
            .tick_ms
            .0
            .iter()
            .filter(|ms| **ms > 1000.0)
            .count(),
        last_tick_age_ms: diagnostics
            .last_tick
            .map(|at| at.elapsed().as_secs_f64() * 1000.0),
        schedule_delay_ms: diagnostics.schedule_delay_ms,
        projection: diagnostics
            .roles
            .get(&role.id)
            .map(|samples| ProjectionView {
                tick: samples.tick,
                build_ms: samples.build_ms.summary(),
                serialization_ms: samples.serialization_ms.summary(),
                snapshot_bytes: samples.snapshot_bytes,
                visible_queue_packets: samples.visible_queue_packets,
                visible_queue_bytes: samples.visible_queue_bytes,
            }),
    })
}
pub(super) async fn get_diagnostics(
    Path(game_id): Path<Uuid>,
    State(state): State<AppState>,
    AuthQuery(query): AuthQuery<ProjectionQuery>,
) -> ApiResult<View> {
    let games = state.games.read().await;
    let game = games
        .get(&game_id)
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "game_not_found", "game not found"))?;
    Ok(Json(view(game, &query)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn samples_are_bounded_and_percentiles_use_only_the_current_window() {
        let mut samples = Samples::default();
        assert_eq!(samples.summary().p95, None);
        for value in 1..=200 {
            samples.push(value as f64);
        }
        let result = samples.summary();
        assert_eq!(result.samples, WINDOW);
        assert_eq!(result.last, Some(200.0));
        assert_eq!(result.p95, Some(194.0));
    }
    #[test]
    fn committed_rates_use_actual_progress_and_reset_without_counting_processing() {
        let mut diagnostics = Diagnostics::default();
        let start = Instant::now();
        diagnostics.record_tick(10.0, 2.0);
        assert_eq!(
            diagnostics.committed_rate(&diagnostics.committed_frames, start),
            None
        );
        diagnostics.record_commit(start, 1, 0.25 / 60.0);
        assert_eq!(diagnostics.committed_intervals_ms.summary().samples, 0);
        // Maintenance and rolled-back frames never call record_commit with advancing progress.
        diagnostics.record_commit(start + Duration::from_millis(10), 0, 0.0);
        diagnostics.record_commit(start + Duration::from_millis(20), 1, 0.25 / 60.0);
        assert_eq!(
            diagnostics.committed_rate(
                &diagnostics.committed_frames,
                start + Duration::from_millis(20)
            ),
            Some(50.0)
        );
        assert_eq!(
            diagnostics.committed_rate(
                &diagnostics.committed_seconds,
                start + Duration::from_millis(20)
            ),
            Some(0.25 / 60.0 * 50.0)
        );
        assert_eq!(
            diagnostics.committed_rate(
                &diagnostics.committed_frames,
                start + Duration::from_millis(40)
            ),
            Some(25.0)
        );
        assert_eq!(
            diagnostics.committed_rate(
                &diagnostics.committed_frames,
                start + Duration::from_secs(4)
            ),
            None
        );
        for index in 2..=200 {
            diagnostics.record_commit(start + Duration::from_millis(index * 20), 1, 1.0);
        }
        assert_eq!(diagnostics.committed_intervals_ms.summary().samples, WINDOW);
        assert_eq!(diagnostics.committed_seconds.0.len(), WINDOW);
        diagnostics.reset_rate();
        assert_eq!(diagnostics.committed_intervals_ms.summary().samples, 0);
        assert!(diagnostics.last_commit.is_none());
    }
    #[test]
    fn diagnostics_are_role_scoped_and_do_not_change_projection_bytes() {
        let mut game = authority_tests::game_for(regional_campaign_scenario());
        let player = Uuid::from_u128(800);
        let role_id = Uuid::from_u128(211);
        game.roles.get_mut(&role_id).unwrap().owner = Some(player);
        let role = game.roles[&role_id].clone();
        let expected = serde_json::to_vec(
            &game
                .simulation
                .projection_for(role.location_unit_id, role.side),
        )
        .unwrap();
        assert_eq!(projection_bytes(&mut game, &role).unwrap(), expected);
        let query = ProjectionQuery {
            lease_generation: game.roles[&role_id].lease_generation,
            player_id: player,
            role_id,
            after_sequence: None,
        };
        let result = view(&game, &query).unwrap();
        assert_eq!(result.projection.unwrap().snapshot_bytes, expected.len());
        let forbidden = ProjectionQuery {
            role_id: Uuid::from_u128(201),
            ..query
        };
        assert_eq!(
            view(&game, &forbidden).err().unwrap().0,
            StatusCode::FORBIDDEN
        );
        // Reading diagnostics cannot advance time or alter knowledge.
        assert_eq!(projection_bytes(&mut game, &role).unwrap(), expected);
    }
}
