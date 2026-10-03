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

impl super::Simulation {
    pub fn last_step_timings(&self) -> StepTimings {
        self.last_step_timings
    }
}
