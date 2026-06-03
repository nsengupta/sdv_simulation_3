//! L4 ingress demux: [`FsmEvent`] → per-zone [`HeadlampMessage`] (and powertrain/visibility apply).

use std::time::Instant;

use crate::fsm::{FsmEvent, FsmState};
use crate::vehicle_state::{
    HeadlampMessage, HeadlampOutcome, HeadlampState, VehicleContext,
};

/// Zone layer output for one mailbox event (headlamp first; extend for other zones in ADR-6).
#[derive(Debug)]
pub struct ZoneTurnResult {
    pub ctx: VehicleContext,
    pub headlamp_outcomes: Vec<HeadlampOutcome>,
    pub headlamp_before: HeadlampState,
}

/// Apply ingress to L1 zones. Does not run the operational FSM (L2).
pub fn zone_turn(
    ctx: &VehicleContext,
    event: &FsmEvent,
    current_state: &FsmState,
    now: Instant,
) -> ZoneTurnResult {
    let headlamp_before = ctx.headlamp.state;
    let prev_headlamp = ctx.headlamp.state;
    let mut next = ctx.clone();
    let mut headlamp_outcomes = Vec::new();

    match event {
        FsmEvent::UpdateRpm(rpm) => {
            next.powertrain.apply_rpm(*rpm);
            next.powertrain.refresh_speed();
            if *current_state == FsmState::Off {
                next.powertrain.freeze_standstill();
            }
        }
        FsmEvent::UpdateAmbientLux(lux) => {
            next.visibility.apply_lux(*lux);
            headlamp_outcomes.extend(next.headlamp.apply(
                HeadlampMessage::AmbientLux(*lux),
                prev_headlamp,
                now,
            ));
        }
        FsmEvent::FrontHeadlampOnAck => {
            headlamp_outcomes.extend(
                next.headlamp
                    .apply(HeadlampMessage::AckOn, prev_headlamp, now),
            );
        }
        FsmEvent::FrontHeadlampOffAck => {
            headlamp_outcomes.extend(
                next.headlamp
                    .apply(HeadlampMessage::AckOff, prev_headlamp, now),
            );
        }
        FsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            headlamp_outcomes.extend(next.headlamp.apply(
                HeadlampMessage::ActuationIncomplete {
                    direction: *direction,
                    cause: *cause,
                },
                prev_headlamp,
                now,
            ));
        }
        FsmEvent::TimerTick => {
            headlamp_outcomes.extend(
                next.headlamp
                    .apply(HeadlampMessage::TimerTick, prev_headlamp, now),
            );
        }
        FsmEvent::PowerOn | FsmEvent::PowerOff => {}
    }

    ZoneTurnResult {
        ctx: next,
        headlamp_outcomes,
        headlamp_before,
    }
}
