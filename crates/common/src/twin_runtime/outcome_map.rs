//! L4: map L1 zone outcomes to L2 [`DomainAction`] for the actuation path (shim until ADR-6).

use crate::fsm::DomainAction;
use crate::vehicle_state::HeadlampOutcome;

pub fn headlamp_outcomes_to_domain_actions(
    outcomes: impl IntoIterator<Item = HeadlampOutcome>,
) -> Vec<DomainAction> {
    outcomes
        .into_iter()
        .filter_map(|o| match o {
            HeadlampOutcome::RequestOn => Some(DomainAction::RequestFrontHeadlampOn),
            HeadlampOutcome::RequestOff => Some(DomainAction::RequestFrontHeadlampOff),
            HeadlampOutcome::LogWarning(msg) => Some(DomainAction::LogWarning(msg)),
        })
        .collect()
}
