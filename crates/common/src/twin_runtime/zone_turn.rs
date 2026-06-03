//! L4 ingress demux: [`FsmEvent`] → per-zone [`HeadlampMessage`] (and powertrain/visibility apply).

use std::time::Instant;

use crate::fsm::{FsmEvent, FsmState};
use crate::vehicle_state::{
    HeadlampMessage, HeadlampOutcome, HeadlampState,
    HeadlampZoneReply, VehicleContext,
};

/// Zone layer output for one mailbox event (headlamp first; extend for other zones in ADR-6).
#[derive(Debug)]
pub struct ZoneTurnResult {
    pub ctx: VehicleContext,
    pub headlamp_outcomes: Vec<HeadlampOutcome>,
    pub headlamp_before: HeadlampState,
}

pub(crate) fn fsm_event_headlamp_message(event: &FsmEvent) -> Option<HeadlampMessage> {
    match event {
        FsmEvent::UpdateAmbientLux(lux) => Some(HeadlampMessage::AmbientLux(*lux)),
        FsmEvent::FrontHeadlampOnAck => Some(HeadlampMessage::AckOn),
        FsmEvent::FrontHeadlampOffAck => Some(HeadlampMessage::AckOff),
        FsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            Some(HeadlampMessage::ActuationIncomplete {
                direction: *direction,
                cause: *cause,
            })
        }
        FsmEvent::TimerTick => Some(HeadlampMessage::TimerTick),
        FsmEvent::UpdateRpm(_) | FsmEvent::PowerOn | FsmEvent::PowerOff => None,
    }
}

/// Apply ingress to L1 zones. Does not run the operational FSM (L2).
///
/// `headlamp_reply`: when `Some`, twinlet already applied this event's zone message (brain path).
pub fn zone_turn(
    ctx: &VehicleContext,
    event: &FsmEvent,
    current_state: &FsmState,
    now: Instant,
    headlamp_reply: Option<HeadlampZoneReply>,
) -> ZoneTurnResult {
    let headlamp_before = ctx.headlamp.state;
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
            let zone_reply = headlamp_reply.unwrap_or_else(|| {
                ctx.headlamp
                    .on_receiving_message(HeadlampMessage::AmbientLux(*lux), now)
            });
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::FrontHeadlampOnAck => {
            let zone_reply = headlamp_reply
                .unwrap_or_else(|| ctx.headlamp.on_receiving_message(HeadlampMessage::AckOn, now));
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::FrontHeadlampOffAck => {
            let zone_reply = headlamp_reply
                .unwrap_or_else(|| ctx.headlamp.on_receiving_message(HeadlampMessage::AckOff, now));
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            let zone_reply = headlamp_reply.unwrap_or_else(|| {
                ctx.headlamp.on_receiving_message(
                    HeadlampMessage::ActuationIncomplete {
                        direction: *direction,
                        cause: *cause,
                    },
                    now,
                )
            });
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::TimerTick => {
            let zone_reply = headlamp_reply.unwrap_or_else(|| {
                ctx.headlamp.on_receiving_message(HeadlampMessage::TimerTick, now)
            });
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::PowerOn | FsmEvent::PowerOff => {}
    }

    ZoneTurnResult {
        ctx: next,
        headlamp_outcomes,
        headlamp_before,
    }
}
