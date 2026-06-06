//! L4 ingress demux: [`FsmEvent`] → per-zone messages (and in-process L1 where no twinlet yet).

use std::time::Instant;

use crate::fsm::{FsmEvent, FsmState};
use crate::twin_runtime::zone_replies::ZoneReplies;
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
        FsmEvent::UpdateRpm(_)
        | FsmEvent::PowerOn
        | FsmEvent::PowerOff
        | FsmEvent::TimerTick
        | FsmEvent::Internal(_) => None,
    }
}

fn merge_headlamp_for_message(
    ctx: &VehicleContext,
    message: HeadlampMessage,
    now: Instant,
    tell_back: Option<&HeadlampZoneReply>,
) -> HeadlampZoneReply {
    tell_back.cloned().unwrap_or_else(|| ctx.headlamp.on_receiving_message(message, now))
}

/// Apply ingress to L1 zones. Does not run the operational FSM (L2).
pub fn zone_turn(
    ctx: &VehicleContext,
    event: &FsmEvent,
    current_state: &FsmState,
    now: Instant,
    zone_replies: &ZoneReplies,
) -> ZoneTurnResult {
    let headlamp_before = ctx.headlamp.state;
    let mut next = ctx.clone();
    let mut headlamp_outcomes = Vec::new();
    let ingress = zone_replies.headlamp.ingress.as_ref();

    match event {
        FsmEvent::UpdateRpm(rpm) => {
            next.powertrain.apply_rpm(*rpm);
            next.powertrain.refresh_speed();
            if *current_state == FsmState::Off {
                next.powertrain.freeze_standstill();
            }
        }
        FsmEvent::UpdateAmbientLux(lux) => {
            // visibility in-process; headlamp via twinlet tell-back merged from `ingress` embed
            next.visibility.apply_lux(*lux);
            let zone_reply = merge_headlamp_for_message(
                ctx,
                HeadlampMessage::AmbientLux(*lux),
                now,
                ingress,
            );
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::FrontHeadlampOnAck => {
            let zone_reply =
                merge_headlamp_for_message(ctx, HeadlampMessage::AckOn, now, ingress);
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::FrontHeadlampOffAck => {
            let zone_reply =
                merge_headlamp_for_message(ctx, HeadlampMessage::AckOff, now, ingress);
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::FrontHeadlampActuationIncomplete { direction, cause } => {
            let zone_reply = merge_headlamp_for_message(
                ctx,
                HeadlampMessage::ActuationIncomplete {
                    direction: *direction,
                    cause: *cause,
                },
                now,
                ingress,
            );
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::TimerTick => {
            let zone_reply =
                merge_headlamp_for_message(ctx, HeadlampMessage::TimerTick, now, ingress);
            next.headlamp = zone_reply.ctx;
            headlamp_outcomes.extend(zone_reply.outcomes);
        }
        FsmEvent::PowerOn | FsmEvent::PowerOff | FsmEvent::Internal(_) => {}
    }

    ZoneTurnResult {
        ctx: next,
        headlamp_outcomes,
        headlamp_before,
    }
}
