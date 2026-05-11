//! Compatibility contract tests ensuring `fsm::engine` remains a stable shim
//! over `engine::op_strategy::transition_map`.

use crate::engine::op_strategy::transition_map;
use crate::fsm::{output, transition, FsmEvent, FsmState, LightingState, VehicleContext};
use std::time::Instant;

fn valid_twin_context() -> VehicleContext {
    VehicleContext {
        rpm: 0,
        speed: 0,
        fuel_level: 85,
        oil_pressure: 30,
        tyre_pressure_ok: true,
        ambient_lux: 100,
        lighting_state: LightingState::Off,
        lighting_ack_pending_since: None,
    }
}

#[test]
fn given_driving_when_high_rpm_then_shim_and_strategy_transition_match() {
    let now = Instant::now();
    let mut ctx = valid_twin_context();
    ctx.rpm = 6500;

    let via_shim = transition(&FsmState::Driving, &FsmEvent::UpdateRpm(6500), &ctx, now);
    let via_strategy = transition_map::transition(&FsmState::Driving, &FsmEvent::UpdateRpm(6500), &ctx, now);

    assert_eq!(via_shim, via_strategy);
}

#[test]
fn given_warning_recovery_when_transition_occurs_then_shim_and_strategy_output_match() {
    let old_state = FsmState::Warning(Instant::now());
    let new_state = FsmState::Driving;

    let via_shim = output(&old_state, &new_state);
    let via_strategy = transition_map::output(&old_state, &new_state);

    assert_eq!(via_shim, via_strategy);
}
