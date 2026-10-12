//! Exact fractional game time for a steady 60 Hz wall-clock loop.
use super::*;

pub const TICKS_PER_SECOND: u32 = 60;
const UNITS_PER_SECOND: u64 = 240;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Resource)]
pub(super) struct FrameClock {
    pub units: u64,
    pub delta_units: u16,
    pub frame: u64,
}
impl FrameClock {
    pub fn seconds(self) -> f64 {
        self.units as f64 / UNITS_PER_SECOND as f64
    }
    pub fn previous_units(self) -> u64 {
        self.units.saturating_sub(u64::from(self.delta_units))
    }
    pub fn crossed_second(self) -> bool {
        self.units / UNITS_PER_SECOND > self.previous_units() / UNITS_PER_SECOND
    }
    pub fn valid(self) -> bool {
        matches!(self.delta_units, 1 | 4 | 8 | 20 | 40 | 120 | 240)
    }
}

pub(super) fn game_second_advanced(clock: Option<Res<FrameClock>>) -> bool {
    clock.is_none_or(|clock| clock.crossed_second())
}

impl Simulation {
    fn prepare_frame(&mut self, units: u16) -> Result<(), String> {
        if !matches!(units, 1 | 4 | 8 | 20 | 40 | 120 | 240) {
            return Err("unsupported fractional frame duration".into());
        }
        if !self.world.contains_resource::<FrameClock>() {
            self.world.insert_resource(FrameClock {
                units: self.radio_tick().saturating_mul(UNITS_PER_SECOND),
                delta_units: units,
                frame: 0,
            });
        }
        self.world.resource_mut::<FrameClock>().delta_units = units;
        Ok(())
    }
    /// One wall-clock frame: scale / 60 game seconds, represented exactly.
    pub fn step_frame(&mut self, scale: f64) -> Result<(), String> {
        if ![0.25, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0].contains(&scale) {
            return Err("unsupported game speed".into());
        }
        self.prepare_frame((scale * 4.0) as u16)?;
        self.step_current_clock();
        Ok(())
    }
    pub fn frame_number(&self) -> Option<u64> {
        self.world
            .get_resource::<FrameClock>()
            .map(|clock| clock.frame)
    }
    pub fn elapsed_game_seconds(&self) -> f64 {
        self.world
            .get_resource::<FrameClock>()
            .map_or(self.radio_tick() as f64, |clock| clock.seconds())
    }
    pub fn advance_reporting_frame(&mut self, scale: f64) -> Result<(), String> {
        if !self.mission_complete() {
            return Err("reporting frame requires a completed mission".into());
        }
        if ![0.25, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0].contains(&scale) {
            return Err("unsupported game speed".into());
        }
        self.prepare_frame((scale * 4.0) as u16)?;
        let crossed = {
            let mut clock = self.world.resource_mut::<FrameClock>();
            clock.units = clock.units.saturating_add(u64::from(clock.delta_units));
            clock.frame = clock.frame.saturating_add(1);
            clock.crossed_second()
        };
        if crossed {
            self.advance_reporting_second();
        }
        Ok(())
    }
}
