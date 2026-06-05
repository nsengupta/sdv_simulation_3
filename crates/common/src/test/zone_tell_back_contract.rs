//! Unit tests for tell-back retry / synthetic embed policy.

use crate::fsm::DomainAction;
use crate::twin_runtime::constants::ZONE_TELL_BACK_MAX_RETRIES;
use crate::twin_runtime::zone_tell_back::{
    on_tell_back_timeout, synthetic_unresponsive_headlamp_reply, TellBackTimeoutOutcome, TellBackWait,
};
use crate::twin_runtime::twin_turn;
use crate::vehicle_state::{HeadlampContext, HeadlampOutcome};
use crate::fsm::{FsmEvent, FsmState};
use std::time::Instant;

#[test]
fn tell_back_wait_retries_then_synthesizes_unresponsive_embed() {
    let ctx = HeadlampContext::default();
    let mut wait = TellBackWait::new(7);

    for remaining in (0..ZONE_TELL_BACK_MAX_RETRIES).rev() {
        assert_eq!(wait.retries_remaining, remaining + 1);
        match on_tell_back_timeout(&ctx, wait) {
            TellBackTimeoutOutcome::Retry(next) => wait = next,
            TellBackTimeoutOutcome::Exhausted(_) => panic!("expected retry"),
        }
    }

    match on_tell_back_timeout(&ctx, wait) {
        TellBackTimeoutOutcome::Exhausted(reply) => {
            assert_eq!(reply.ctx, ctx);
            assert!(matches!(
                &reply.outcomes[0],
                HeadlampOutcome::LogWarning(msg) if msg.contains("unresponsive")
            ));
        }
        TellBackTimeoutOutcome::Retry(_) => panic!("expected synthetic embed"),
    }
}

#[test]
fn synthetic_unresponsive_embed_surfaces_log_warning_on_commit() {
    let t0 = Instant::now();
    let ctx = HeadlampContext::default();
    let synthetic = synthetic_unresponsive_headlamp_reply(&ctx);
    let result = twin_turn::commit_brain_turn(
        &FsmState::Driving,
        &driving_ctx(),
        &FsmEvent::TimerTick,
        t0,
        Some(synthetic),
        None,
    );
    assert!(
        result.actions.iter().any(|a| matches!(a, DomainAction::LogWarning(msg) if msg.contains("unresponsive"))),
        "ledger path must carry unresponsive warning, got {:?}",
        result.actions
    );
}

fn driving_ctx() -> crate::vehicle_state::VehicleContext {
    use crate::vehicle_physics::{LUX_ON_THRESHOLD, RPM_DRIVING_THRESHOLD};
    let mut ctx = crate::vehicle_state::VehicleContext::default();
    ctx.visibility.ambient_lux = LUX_ON_THRESHOLD;
    ctx.powertrain.apply_rpm(RPM_DRIVING_THRESHOLD + 100);
    ctx.powertrain.refresh_speed();
    ctx
}

#[tokio::test]
async fn given_silent_headlamp_when_timer_tick_then_ledger_records_unresponsive_warning() {
    use crate::fsm::FsmEvent;
    use crate::test::ActorGuard;
    use crate::twin_runtime::constants::{ZONE_TELL_BACK_ATTEMPT_COUNT, ZONE_TELL_BACK_WAIT};
    use crate::twin_runtime::controller::vehicle_controller::VehicleControllerRuntimeOptions;
    use crate::{PublishedDomainAction, PublishedFsmEvent, VehicleController};
    use tokio::sync::mpsc;

    let (transition_tx, mut rx) = mpsc::channel(8);
    let runtime_options = VehicleControllerRuntimeOptions {
        transition_tx: Some(transition_tx),
        test_silent_headlamp: true,
        ..VehicleControllerRuntimeOptions::default()
    };

    let (controller, handle) = VehicleController::install_and_start_with_options(
        "ZONE-TELL-BACK-01".to_string(),
        runtime_options,
    )
    .await
    .expect("start actor");
    let _guard = ActorGuard {
        addr: controller.get_actor_ref().clone(),
        handle,
    };

    controller.send_power_on().await.expect("power on");
    let _ = rx.recv().await.expect("power on row");

    controller
        .submit_fsm_event(FsmEvent::TimerTick)
        .await
        .expect("timer tick");

    let wait_budget = ZONE_TELL_BACK_WAIT
        .saturating_mul(ZONE_TELL_BACK_ATTEMPT_COUNT as u32)
        .saturating_add(std::time::Duration::from_millis(100));
    let record = tokio::time::timeout(wait_budget, rx.recv())
        .await
        .expect("ledger row within tell-back retry budget")
        .expect("timer tick row");

    assert_eq!(record.event, PublishedFsmEvent::TimerTick);
    assert!(
        record
            .actions
            .iter()
            .any(|action| matches!(action, PublishedDomainAction::LogWarning(msg) if msg.contains("unresponsive"))),
        "transition log must record headlamp unresponsive, got {:?}",
        record.actions
    );
}
