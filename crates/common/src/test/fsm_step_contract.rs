//! Unit tests for the FSM step contract (`step`).

use crate::digital_twin::DigitalTwinCar;
use crate::fsm::{step, DomainAction, FsmEvent, FsmState, VehicleContext};
use std::time::{Duration, Instant};

fn valid_twin_context() -> VehicleContext {
    VehicleContext {
        rpm: 0,
        speed: 0,
        fuel_level: 85,
        oil_pressure: 30,
        tyre_pressure_ok: true,
        ambient_lux: 100,
        lighting_state: crate::fsm::LightingState::Off,
        lighting_ack_pending_since: None,
    }
}

#[test]
fn test_step_derive_ctx_and_warning_flow() {
    let mut current_ctx = valid_twin_context();
    let mut current_state = FsmState::Idle;

    let warmup = step(
        &current_state,
        &current_ctx,
        &FsmEvent::UpdateRpm(1200),
        Instant::now(),
    );
    assert_eq!(warmup.next_state, FsmState::Driving);
    assert_eq!(warmup.modified_ctx.rpm, 1200);

    current_state = warmup.next_state;
    current_ctx = warmup.modified_ctx;

    let warning = step(
        &current_state,
        &current_ctx,
        &FsmEvent::UpdateRpm(6500),
        Instant::now(),
    );
    assert_eq!(warning.modified_ctx.rpm, 6500);
    assert!(matches!(warning.next_state, FsmState::Warning(_)));
    assert!(warning.actions.contains(&DomainAction::StartBuzzer));
    assert!(warning
        .actions
        .contains(&DomainAction::LogWarning("Overspeed detected!".to_string())));
}

#[test]
fn test_step_standard_commute_flow() {
    let mut car = DigitalTwinCar {
        identity: "NASHIK-VC-001".to_string(),
        current_state: FsmState::Off,
        context: valid_twin_context(),
    };

    let sequence = vec![
        (FsmEvent::PowerOn, FsmState::Idle),
        (FsmEvent::UpdateRpm(1500), FsmState::Driving),
        (FsmEvent::UpdateSpeed(50), FsmState::Driving),
        (FsmEvent::UpdateSpeed(0), FsmState::Idle),
        (FsmEvent::PowerOff, FsmState::Off),
    ];

    for (event, expected_state) in sequence {
        let result = step(&car.current_state, &car.context, &event, Instant::now());
        car.current_state = result.next_state;
        car.context = result.modified_ctx;
        assert_eq!(car.current_state, expected_state, "event={event:?}");
    }
}

#[test]
fn test_step_warning_recovery_on_tick_uses_passed_time() {
    let base = Instant::now();
    let mut ctx = valid_twin_context();
    ctx.rpm = 3000;
    ctx.speed = 10;

    let warning_state = FsmState::Warning(base);

    let early = step(
        &warning_state,
        &ctx,
        &FsmEvent::TimerTick,
        base + Duration::from_secs(2),
    );
    assert!(matches!(early.next_state, FsmState::Warning(_)));

    let recovered = step(
        &warning_state,
        &ctx,
        &FsmEvent::TimerTick,
        base + Duration::from_secs(6),
    );
    assert_eq!(recovered.next_state, FsmState::Driving);
    assert!(recovered.actions.contains(&DomainAction::StopBuzzer));
}
