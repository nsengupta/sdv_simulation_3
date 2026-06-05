//! L4 runtime timing — brain↔zone coordination (not L0 physical laws).
//!
//! Launch-time configuration is planned; constants are defaults until then.

use std::time::Duration;

/// How long the brain waits for one zone twinlet tell-back before re-tell or synthetic embed.
///
/// Not headlamp ACK semantics — see [`crate::vehicle_physics::FRONT_HEADLAMP_ON_ACK_WAIT`]
/// (zone-internal, Headlamp actor).
#[cfg(not(test))]
pub const ZONE_TELL_BACK_WAIT: Duration = Duration::from_millis(500);

#[cfg(test)]
pub const ZONE_TELL_BACK_WAIT: Duration = Duration::from_millis(50);

/// Re-tells after a tell-back timeout before synthesizing an unresponsive embed.
/// Total tell attempts per wait cycle = `1 + ZONE_TELL_BACK_MAX_RETRIES` (design decision).
pub const ZONE_TELL_BACK_MAX_RETRIES: u8 = 2;

pub const ZONE_TELL_BACK_ATTEMPT_COUNT: u32 = ZONE_TELL_BACK_MAX_RETRIES as u32 + 1;
