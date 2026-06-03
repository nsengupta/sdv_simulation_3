//! L4 turn: [`zone_turn`] then L2 [`step`], merge zone outcomes into [`DomainAction`].

use std::time::Instant;

use crate::fsm::{step, DomainAction, FsmEvent, FsmState, StepResult};
use crate::twin_runtime::headlamp_actor::{apply_headlamp_zone, HeadlampActorVocabulary};
use crate::twin_runtime::outcome_map::headlamp_outcomes_to_domain_actions;
use crate::twin_runtime::zone_turn::{fsm_event_headlamp_message, zone_turn};
use crate::vehicle_state::{HeadlampMessage, HeadlampZoneReply, VehicleContext};
use ractor::{ActorProcessingErr, ActorRef};

/// Full deterministic turn (pure tests — headlamp applied locally in [`zone_turn`]).
pub fn twin_turn(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
) -> StepResult {
    twin_turn_with_headlamp_replies(current_state, current_ctx, event, now, None, None)
}

/// Brain path: RPC to headlamp twinlet, then one [`twin_turn_with_headlamp_replies`].
pub async fn brain_twin_turn(
    headlamp: &ActorRef<HeadlampActorVocabulary>,
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
) -> Result<StepResult, ActorProcessingErr> {
    let headlamp_reply = match fsm_event_headlamp_message(event) {
        Some(msg) => Some(apply_headlamp_zone(headlamp, msg, now).await?),
        None => None,
    };
    let ignition_off_reply = if fsm_step_lands_off(current_state, current_ctx, event, now, headlamp_reply.as_ref())
    {
        Some(
            apply_headlamp_zone(headlamp, HeadlampMessage::ResetForIgnitionOff, now).await?,
        )
    } else {
        None
    };
    Ok(twin_turn_with_headlamp_replies(
        current_state,
        current_ctx,
        event,
        now,
        headlamp_reply,
        ignition_off_reply,
    ))
}

/// One brain FSM event. `headlamp_reply` / `ignition_off_reply`: when `Some`, twinlet already
/// applied that message (A→C bridge — see milestone doc); `None` → local [`HeadlampContext::on_receiving_message`].
fn twin_turn_with_headlamp_replies(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
    headlamp_reply: Option<HeadlampZoneReply>,
    ignition_off_reply: Option<HeadlampZoneReply>,
) -> StepResult {
    let zone = zone_turn(current_ctx, event, current_state, now, headlamp_reply);
    let mut result = step(current_state, &zone.ctx, event, now);

    let mut headlamp_outcomes = zone.headlamp_outcomes;
    if matches!(result.next_state, FsmState::Off) {
        let zone_reply = ignition_off_reply.unwrap_or_else(|| {
            result.modified_ctx.headlamp.on_receiving_message(
                HeadlampMessage::ResetForIgnitionOff,
                now,
            )
        });
        result.modified_ctx.headlamp = zone_reply.ctx;
        headlamp_outcomes.extend(zone_reply.outcomes);
    }

    let zone_actions = headlamp_outcomes_to_domain_actions(headlamp_outcomes);
    result.actions = zone_actions
        .into_iter()
        .chain(result.actions)
        .collect();

    let recorded_actions: Vec<DomainAction> = result
        .actions
        .iter()
        .filter(|action| !matches!(action, DomainAction::EnterMode(_)))
        .cloned()
        .collect();

    result.transition_record.actions = recorded_actions;
    result.transition_record.current_ctx = result.modified_ctx.clone();

    result
}

fn fsm_step_lands_off(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
    headlamp_reply: Option<&HeadlampZoneReply>,
) -> bool {
    let zone = zone_turn(
        current_ctx,
        event,
        current_state,
        now,
        headlamp_reply.cloned(),
    );
    matches!(
        step(current_state, &zone.ctx, event, now).next_state,
        FsmState::Off
    )
}
