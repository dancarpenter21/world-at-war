//! Observational timings. Never serialized into authoritative projections or used by simulation systems.
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct StepTimings {
    pub total_ms: f64,
    pub ecs_ms: f64,
    pub interference_ms: f64,
    pub transport_ms: f64,
    pub transmission_ms: f64,
    pub network_advance_ms: f64,
    pub reassembly_ms: f64,
    pub reports_ms: f64,
}

/// Aggregate retained state. Payload lengths exclude allocator/container overhead.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct RetentionStatistics {
    pub network: c3mesh::HistoryStatistics,
    pub transport_messages: usize,
    pub transport_fragments: usize,
    pub transport_completed_ids: usize,
    pub transport_events: usize,
    pub transport_payload_bytes: usize,
    pub pending_network_events: usize,
    pub pending_fragment_events: usize,
}

impl super::Simulation {
    pub fn last_step_timings(&self) -> StepTimings {
        self.last_step_timings
    }

    pub fn retention_statistics(&self) -> RetentionStatistics {
        let mut counts = self.transport.retention_statistics();
        counts.network = self.communications.simulator.history_statistics();
        counts.pending_network_events = self.pending_network_events.len();
        counts.pending_fragment_events = self.pending_fragment_events.len();
        counts
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct SensingStatistics {
    pub candidate_checks: u64,
    pub terrain_checks: u64,
    pub observation_ms: f64,
}
impl super::Simulation {
    /// Private performance trace only; candidate counts must not enter role APIs.
    pub fn sensing_statistics(&self) -> SensingStatistics {
        if self
            .world
            .get_resource::<super::FrameClock>()
            .is_some_and(|clock| !clock.crossed_second())
        {
            return SensingStatistics::default();
        }
        self.world
            .get_resource::<super::sensing::Runtime>()
            .map(|runtime| SensingStatistics {
                candidate_checks: runtime.candidate_count,
                terrain_checks: runtime.terrain_checks,
                observation_ms: runtime.observation_ms,
            })
            .unwrap_or_default()
    }
}
