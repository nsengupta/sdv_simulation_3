//! L4 turn: [`zone_turn`] then L2 [`step`], merge zone outcomes into [`DomainAction`].
//!
//! **Today:** one hop per [`commit_brain_turn`]; [`run_to_quiescence`] for ADR-7 multi-hop turns.
//! One `commit_resolved_turn` helper in the actor should replace split commit/apply paths.
//! See `docs/adr-007-fsm-quiescence-and-cut.md`.

use std::time::Instant;

use crate::fsm::{step, DomainAction, FsmEvent, FsmState, StepResult};
use crate::twin_runtime::detectors::detect_internal_after_hop;
use crate::twin_runtime::outcome_map::headlamp_outcomes_to_domain_actions;
use crate::twin_runtime::zone_turn::zone_turn;
use crate::vehicle_state::{HeadlampMessage, HeadlampZoneReply, VehicleContext};

const MAX_QUIESCENCE_HOPS: usize = 8;

/// One ledger row inside a quiescent turn.
#[derive(Debug, Clone, PartialEq)]
pub struct HopRecord {
    pub event: FsmEvent,
    pub result: StepResult,
}

/// Full turn after 0+ internal hops (ADR-7).
#[derive(Debug, Clone, PartialEq)]
pub struct QuiescentResult {
    pub hops: Vec<HopRecord>,
}

impl QuiescentResult {
    pub fn final_step(&self) -> &StepResult {
        self.hops
            .last()
            .map(|h| &h.result)
            .expect("quiescence requires at least one hop")
    }

    pub fn merged_actions(&self) -> Vec<DomainAction> {
        self.hops
            .iter()
            .flat_map(|h| h.result.actions.clone())
            .collect()
    }
}

/// Full deterministic turn (pure tests — headlamp applied locally in [`zone_turn`]).
pub fn twin_turn(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
) -> StepResult {
    twin_turn_with_headlamp_replies(current_state, current_ctx, event, now, None, None)
}

/// Brain path after tell-back: one [`twin_turn_with_headlamp_replies`] (apply_step / ledger stay in actor).
pub fn commit_brain_turn(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
    headlamp_reply: Option<HeadlampZoneReply>,
    ignition_off_reply: Option<HeadlampZoneReply>,
) -> StepResult {
    twin_turn_with_headlamp_replies(
        current_state,
        current_ctx,
        event,
        now,
        headlamp_reply,
        ignition_off_reply,
    )
}

/// Mandatory quiescence loop (ADR-7): external ingress + detector-synthesized internal hops.
pub fn run_to_quiescence(
    initial_state: &FsmState,
    initial_ctx: &VehicleContext,
    ingress: &FsmEvent,
    now: Instant,
    headlamp_reply: Option<HeadlampZoneReply>,
    ignition_off_reply: Option<HeadlampZoneReply>,
) -> QuiescentResult {
    let mut queue = vec![ingress.clone()];
    let mut state = initial_state.clone();
    let mut ctx = initial_ctx.clone();
    let mut hops = Vec::new();

    while let Some(event) = queue.first().cloned() {
        if hops.len() >= MAX_QUIESCENCE_HOPS {
            break;
        }
        queue.remove(0);

        let is_first = hops.is_empty();
        let result = apply_single_hop(
            &state,
            &ctx,
            &event,
            now,
            if is_first {
                headlamp_reply.clone()
            } else {
                None
            },
            if is_first {
                ignition_off_reply.clone()
            } else {
                None
            },
        );

        if let Some(internal) = detect_internal_after_hop(&result.next_state, &result.modified_ctx) {
            queue.push(internal);
        }

        state = result.next_state.clone();
        ctx = result.modified_ctx.clone();
        hops.push(HopRecord { event, result });
    }

    QuiescentResult { hops }
}

/// Whether this event **enters** [`FsmState::Off`] from a powered state after zone demux.
pub(crate) fn fsm_step_lands_off(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
    headlamp_reply: Option<&HeadlampZoneReply>,
) -> bool {
    if *current_state == FsmState::Off {
        return false;
    }
    if matches!(event, FsmEvent::Internal(_)) {
        return matches!(
            step(current_state, current_ctx, event, now).next_state,
            FsmState::Off
        );
    }
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

fn apply_single_hop(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
    headlamp_reply: Option<HeadlampZoneReply>,
    ignition_off_reply: Option<HeadlampZoneReply>,
) -> StepResult {
    if matches!(event, FsmEvent::Internal(_)) {
        apply_internal_hop(current_state, current_ctx, event, now)
    } else {
        twin_turn_with_headlamp_replies(
            current_state,
            current_ctx,
            event,
            now,
            headlamp_reply,
            ignition_off_reply,
        )
    }
}

fn apply_internal_hop(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
) -> StepResult {
    step(current_state, current_ctx, event, now)
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
