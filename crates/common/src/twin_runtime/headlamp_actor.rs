//! L4 headlamp twinlet — [`apply_headlamp_zone`] RPC; child runs [`HeadlampContext::on_receiving_message`].

use async_trait::async_trait;
use ractor::concurrency::Duration;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};
use std::time::Instant;

use crate::vehicle_state::{HeadlampContext, HeadlampMessage, HeadlampZoneReply};

/// RPC envelope for [`apply_headlamp_zone`].
#[derive(Debug)]
pub struct HeadlampActorVocabulary {
    pub message: HeadlampMessage,
    pub now: Instant,
    pub reply: RpcReplyPort<HeadlampZoneReply>,
}

#[derive(Default)]
pub struct HeadlampActor;

#[async_trait]
impl Actor for HeadlampActor {
    type Msg = HeadlampActorVocabulary;
    type State = HeadlampContext;
    type Arguments = HeadlampContext;

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        args: Self::Arguments,
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(args)
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        HeadlampActorVocabulary {
            message,
            now,
            reply,
        }: Self::Msg,
        state: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        let zone_reply = state.on_receiving_message(message, now);
        *state = zone_reply.ctx.clone();
        if !reply.is_closed() {
            reply.send(zone_reply).map_err(|e| {
                std::io::Error::other(format!("HeadlampZoneReply reply: {e:?}"))
            })?;
        }
        Ok(())
    }
}

const HEADLAMP_RPC_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn apply_headlamp_zone(
    actor: &ActorRef<HeadlampActorVocabulary>,
    message: HeadlampMessage,
    now: Instant,
) -> Result<HeadlampZoneReply, ActorProcessingErr> {
    use ractor::rpc::CallResult;

    match actor
        .call(
            |reply| HeadlampActorVocabulary {
                message,
                now,
                reply,
            },
            Some(HEADLAMP_RPC_TIMEOUT),
        )
        .await
        .map_err(ActorProcessingErr::from)?
    {
        CallResult::Success(zone_reply) => Ok(zone_reply),
        CallResult::Timeout => Err(ActorProcessingErr::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "apply_headlamp_zone timed out",
        ))),
        CallResult::SenderError => Err(ActorProcessingErr::from(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "apply_headlamp_zone reply port closed",
        ))),
    }
}
