//! Unit tests for the FSM spec (`transition` / `output`).

use crate::fsm::{output, transition, FsmAction, FsmEvent, FsmState, VehicleContext};
use std::time::{Duration, Instant};

/// Healthy `VehicleContext` matching a valid digital twin (same values as `VehicleContext::default()`).
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
fn test_transition_and_output_high_rpm_warning() {
    let ctx = valid_twin_context();
    let now = Instant::now();
    let driving = transition(&FsmState::Idle, &FsmEvent::UpdateRpm(1200), &ctx, now);
    assert_eq!(driving, FsmState::Driving);

    let warning = transition(&driving, &FsmEvent::UpdateRpm(6500), &ctx, now);
    assert!(matches!(warning, FsmState::Warning(_)));

    let actions = output(&FsmState::Driving, &warning);
    assert!(actions.contains(&FsmAction::StartBuzzer));
    assert!(actions.contains(&FsmAction::LogWarning("Overspeed detected!".to_string())));
}

#[test]
fn test_transition_standard_commute_flow() {
    let ctx = valid_twin_context();
    let now = Instant::now();
    let mut state = transition(&FsmState::Off, &FsmEvent::PowerOn, &ctx, now);
    assert_eq!(state, FsmState::Idle);

    state = transition(&state, &FsmEvent::UpdateRpm(1500), &ctx, now);
    assert_eq!(state, FsmState::Driving);

    state = transition(&state, &FsmEvent::UpdateSpeed(50), &ctx, now);
    assert_eq!(state, FsmState::Driving);

    state = transition(&state, &FsmEvent::UpdateSpeed(0), &ctx, now);
    assert_eq!(state, FsmState::Idle);

    state = transition(&state, &FsmEvent::PowerOff, &ctx, now);
    assert_eq!(state, FsmState::Off);
}

#[test]
fn test_transition_illegal_shutdown_attempt() {
    let ctx = VehicleContext {
        rpm: 3000,
        speed: 80,
        ..VehicleContext::default()
    };
    let state = transition(&FsmState::Driving, &FsmEvent::PowerOff, &ctx, Instant::now());
    assert_eq!(state, FsmState::Driving);
}

#[test]
fn test_warning_recovery_requires_cooldown_and_low_rpm() {
    let base = Instant::now();
    let warning = FsmState::Warning(base);
    let mut ctx = valid_twin_context();
    ctx.speed = 30;

    // Before cooldown, must stay in Warning.
    ctx.rpm = 3000;
    let early = transition(
        &warning,
        &FsmEvent::TimerTick,
        &ctx,
        base + Duration::from_secs(2),
    );
    assert!(matches!(early, FsmState::Warning(_)));

    // After cooldown but still high RPM, must stay in Warning.
    ctx.rpm = 6200;
    let high_rpm = transition(
        &warning,
        &FsmEvent::TimerTick,
        &ctx,
        base + Duration::from_secs(6),
    );
    assert!(matches!(high_rpm, FsmState::Warning(_)));

    // After cooldown and RPM low enough, recover to Driving.
    ctx.rpm = 3000;
    let recovered = transition(
        &warning,
        &FsmEvent::TimerTick,
        &ctx,
        base + Duration::from_secs(6),
    );
    assert_eq!(recovered, FsmState::Driving);

    let actions = output(&warning, &recovered);
    assert!(actions.contains(&FsmAction::StopBuzzer));
}

#[test]
fn test_warning_recovers_to_idle_when_stationary() {
    let base = Instant::now();
    let warning = FsmState::Warning(base);
    let mut ctx = valid_twin_context();
    ctx.rpm = 2500;
    ctx.speed = 0;

    let recovered = transition(
        &warning,
        &FsmEvent::TimerTick,
        &ctx,
        base + Duration::from_secs(6),
    );
    assert_eq!(recovered, FsmState::Idle);

    let actions = output(&warning, &recovered);
    assert!(actions.contains(&FsmAction::StopBuzzer));
}
