//! L4 turn: [`zone_turn`] then L2 [`step`], merge zone outcomes into [`DomainAction`].

use std::time::Instant;

use crate::fsm::{step, DomainAction, FsmEvent, FsmState, StepResult};
use crate::twin_runtime::outcome_map::headlamp_outcomes_to_domain_actions;
use crate::twin_runtime::zone_turn::zone_turn;
use crate::vehicle_state::{HeadlampMessage, VehicleContext};

/// Full deterministic turn for the virtual car actor and contract tests.
pub fn twin_turn(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
) -> StepResult {
    let zone = zone_turn(current_ctx, event, current_state, now);
    let mut result = step(current_state, &zone.ctx, event, now);

    let mut headlamp_outcomes = zone.headlamp_outcomes;
    if matches!(result.next_state, FsmState::Off) {
        let prev = result.modified_ctx.headlamp.state;
        headlamp_outcomes.extend(result.modified_ctx.headlamp.apply(
            HeadlampMessage::ResetForIgnitionOff,
            prev,
            now,
        ));
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
