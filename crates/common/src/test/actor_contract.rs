//! Actor-oriented contract tests (mailbox -> step -> persistence/emit sequencing).

use crate::digital_twin::DigitalTwinCarVocabulary;
use crate::engine::controller::vehicle_controller::VehicleControllerRuntimeOptions;
use crate::fsm::{FsmEvent, FsmState, LightingState};
use crate::test::{
    expect_actuation_command, inject_matching_ack, inject_matching_nack, install_with_actuation,
    ActorGuard,
};
use crate::{ActuationCommand, PhysicalCarVocabulary, VehicleController, VssSignal};
use ractor::concurrency::Duration;
use tokio::sync::mpsc;

/// Timeout for actor call in contract tests.
const DEFAULT_ACTOR_TIMEOUT: Duration = Duration::from_millis(250);

#[tokio::test]
async fn scenario_raw_transition_records_are_emitted_in_order() {
    let (tx, mut rx) = mpsc::channel(16);

    let runtime_options = VehicleControllerRuntimeOptions {
        transition_tx: Some(tx),
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "SCENARIO-LOGGING-01".to_string(),
        runtime_options,
    )
    .await
    .expect("Failed to start DigitalTwin Actor with sink");
    let actor_ref = controller.get_actor_ref().clone();
    let _guard = ActorGuard {
        addr: actor_ref.clone(),
        handle,
    };

    actor_ref
        .send_message(FsmEvent::PowerOn.into())
        .expect("Failed to send PowerOn stimulus");
    actor_ref
        .send_message(FsmEvent::UpdateRpm(1500).into())
        .expect("Failed to send UpdateRpm stimulus");

    let first = rx.recv().await.expect("Missing first transition record");
    let second = rx.recv().await.expect("Missing second transition record");

    assert_eq!(first.record_seq, 1);
    assert_eq!(first.transition.event, FsmEvent::PowerOn);
    assert_eq!(first.transition.old_state, FsmState::Off);
    assert_eq!(first.transition.next_state, FsmState::Idle);
    assert_eq!(first.transition.current_ctx.powertrain.wheel_rpm.front_left, 0);

    assert_eq!(second.record_seq, 2);
    assert_eq!(second.transition.event, FsmEvent::UpdateRpm(1500));
    assert_eq!(second.transition.old_state, FsmState::Idle);
    assert_eq!(second.transition.next_state, FsmState::Driving);
    assert_eq!(second.transition.current_ctx.powertrain.wheel_rpm.front_left, 1500);

    let twin_snapshot = actor_ref
        .call(
            |port| DigitalTwinCarVocabulary::GetStatus(port),
            Some(DEFAULT_ACTOR_TIMEOUT),
        )
        .await
        .expect("Failed to enqueue GetStatus")
        .expect("Actor failed to respond or timed out during GetStatus request");

    assert_eq!(
        &second.transition.current_ctx, twin_snapshot.context(),
        "emitted current_ctx must match persisted actor context after transition"
    );
}

#[tokio::test]
async fn scenario_log_warning_is_routed_to_diagnostic_sink() {
    // WI-5: a LogWarning domain intent must surface on the diagnostic stream (Warning level),
    // not through the actuation path.
    let (diag_tx, mut diag_rx) = mpsc::unbounded_channel();

    let runtime_options = VehicleControllerRuntimeOptions {
        diagnostic_tx: Some(diag_tx),
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "SCENARIO-WARN-01".to_string(),
        runtime_options,
    )
    .await
    .expect("Failed to start DigitalTwin Actor with diagnostic sink");
    let actor_ref = controller.get_actor_ref().clone();
    let _guard = ActorGuard {
        addr: actor_ref.clone(),
        handle,
    };

    // Drive Off -> Idle -> Driving -> ExtremeOperationWarning (redline), which emits the
    // speed-threshold LogWarning intent.
    for evt in [
        FsmEvent::PowerOn,
        FsmEvent::UpdateRpm(2000),
        FsmEvent::UpdateRpm(7500),
    ] {
        actor_ref
            .send_message(evt.into())
            .expect("Failed to send stimulus");
    }

    let mut saw_warning = false;
    while let Ok(Some(msg)) =
        tokio::time::timeout(Duration::from_millis(250), diag_rx.recv()).await
    {
        if msg.level == crate::DiagnosticLevel::Warning
            && msg.message.contains(crate::SPEED_THRESHOLD_WARNING_MESSAGE)
        {
            saw_warning = true;
            break;
        }
    }

    assert!(
        saw_warning,
        "LogWarning intent should surface as a Warning-level diagnostic"
    );
}

#[tokio::test]
async fn scenario_actuation_ack_round_trip_via_helper() {
    // WI-6 (Q2): observe the outbound command, inject the matching ack, observe the resulting
    // transition — the harness standing in for the future actuation child actor.
    let (controller, mut actuation_rx, _guard) = install_with_actuation("ACT-ACK-01", 16).await;

    controller.send_power_on().await.expect("power on");
    controller
        .submit_physical_car_event(PhysicalCarVocabulary::TelemetryUpdate(VssSignal::AmbientLux(
            20,
        )))
        .await
        .expect("low lux event");

    let command = expect_actuation_command(&mut actuation_rx, Duration::from_millis(250)).await;
    assert!(
        matches!(command, ActuationCommand::SwitchFrontHeadlampOn { .. }),
        "low lux should request the front headlamp ON, got {command:?}"
    );

    inject_matching_ack(&controller, &command).await;

    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(250)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.context().headlamp.state, LightingState::On);
    assert!(snapshot.context().headlamp.ack_pending_since.is_none());
}

#[tokio::test]
async fn scenario_actuation_nack_round_trip_via_helper() {
    let (controller, mut actuation_rx, _guard) = install_with_actuation("ACT-NACK-01", 16).await;

    controller.send_power_on().await.expect("power on");
    controller
        .submit_physical_car_event(PhysicalCarVocabulary::TelemetryUpdate(VssSignal::AmbientLux(
            20,
        )))
        .await
        .expect("low lux event");

    let command = expect_actuation_command(&mut actuation_rx, Duration::from_millis(250)).await;
    assert!(matches!(
        command,
        ActuationCommand::SwitchFrontHeadlampOn { .. }
    ));

    inject_matching_nack(&controller, &command).await;

    // A NACK on the ON request leaves the headlamp Off (the request did not complete).
    let snapshot = controller
        .get_snapshot(Some(Duration::from_millis(250)))
        .await
        .expect("snapshot");
    assert_eq!(snapshot.context().headlamp.state, LightingState::Off);
    assert!(snapshot.context().headlamp.ack_pending_since.is_none());
}
