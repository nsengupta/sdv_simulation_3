//! Consistency rules between reported FSM state and [`VehicleContext`].
//!
//! Defined only in terms of [`FsmState`] and [`VehicleContext`] — no dependency on [`super::DigitalTwinCar`].

use crate::fsm::{FsmState, VehicleContext};

/// 1. Kinetic locking: must not report motion while logically off.
pub(super) fn law_kinetic_locking_holds(state: &FsmState, ctx: &VehicleContext) -> Result<(), String> {
    if *state == FsmState::Off && ctx.powertrain.speed_kph > 0 {
        return Err(format!(
            "Safety Breach: Car is Off but moving at {} km/h",
            ctx.powertrain.speed_kph
        ));
    }
    Ok(())
}

/// 2. Engine logic: driving implies RPM above stall threshold.
pub(super) fn law_rpm_above_threshold_holds(
    state: &FsmState,
    ctx: &VehicleContext,
) -> Result<(), String> {
    if *state == FsmState::Driving && ctx.powertrain.wheel_rpm.front_left < 500 {
        return Err("Logic Breach: State is Driving but RPM is below stall levels".into());
    }
    Ok(())
}
