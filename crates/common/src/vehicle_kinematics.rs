//! Wheel RPM → ground speed (kinematic expectation for the digital twin).
//!
//! Input RPM is the bus-level value from one wheel (or a composite treated as wheel RPM).
//! Future ECUs may publish observed speed separately; until then the twin derives `VehicleContext::powertrain.speed_kph`
//! from `rpm` in [`crate::fsm::step`] via [`calculate_speed_from_rpm`].

use crate::fsm::VehicleContext;

/// Combined multiplier for `(2 * π * radius * 3.6) / 60` with tire radius ~0.303 m.
const RPM_TO_KMH_MULTIPLIER: f64 = 0.114;

/// Convert a single wheel's RPM to ground speed (km/h).
///
/// Assumes perfect traction and a standard tire radius (~0.303 m). Returns the uncapped
/// kinematic value (no artificial 255 km/h limit). [`refresh_context_speed`] stores the
/// rounded value in [`VehicleContext::powertrain.speed_kph`] as `u16`.
///
/// When observed-speed ECUs exist (slip, clutch, gear), they can supply a separate measurement;
/// compare against this kinematic expectation in policy / invariants.
pub fn calculate_speed_from_rpm(rpm: u16) -> f64 {
    let speed_kmh = f64::from(rpm) * RPM_TO_KMH_MULTIPLIER;
    speed_kmh.max(0.0)
}

/// Refresh derived speed (km/h) from the latest wheel RPM.
///
/// Thin wrapper that delegates to the powertrain assembly, which owns the
/// derivation. Retained so existing call sites stay unchanged in Step 1.
pub fn refresh_context_speed(ctx: &mut VehicleContext) {
    ctx.powertrain.refresh_speed();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_rpm_is_zero_kph() {
        assert_eq!(calculate_speed_from_rpm(0), 0.0);
    }

    #[test]
    fn known_rpm_maps_to_expected_kph() {
        assert!((calculate_speed_from_rpm(1000) - 114.0).abs() < f64::EPSILON);
    }

    #[test]
    fn speed_is_monotonic_in_rpm() {
        assert!(calculate_speed_from_rpm(1500) > calculate_speed_from_rpm(500));
        assert!(calculate_speed_from_rpm(3000) > calculate_speed_from_rpm(1500));
    }

    #[test]
    fn high_rpm_is_not_capped_at_255_kph() {
        assert!((calculate_speed_from_rpm(3000) - 342.0).abs() < f64::EPSILON);
    }

    #[test]
    fn refresh_stores_rounded_kph_in_context() {
        let mut ctx = VehicleContext::default();
        ctx.powertrain.wheel_rpm.front_left = 3000;
        refresh_context_speed(&mut ctx);
        assert_eq!(ctx.powertrain.speed_kph, 342);
    }
}
