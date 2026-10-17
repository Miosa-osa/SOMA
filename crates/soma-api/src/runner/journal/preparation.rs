//! The launch path one create took, named the way the journal and `Server-Timing` name it.
//!
//! It lives apart from the record it is carried on because it is the one thing about a create
//! that is not a duration: the record's other fields all say how long something took, and this
//! says which of two very different things happened.

use serde::{Deserialize, Serialize};

/// How a launch came to hold its machine, as the journal and `Server-Timing` name it.
///
/// Every other thing the runner reports about a create is a duration, and a duration says nothing
/// about which path the launch took. A machine the pool prepared is served by claiming it; one
/// that was not is built on the request path, and the two differ by more than the whole of the
/// rest of a create's timing. An operator reading a slow create needs to know which one they have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchPreparation {
    /// The pool held a machine for this Generation and this launch claimed it.
    Prepared,
    /// Nothing was prepared for it, so this launch built its own machine.
    OnDemand,
    /// This launch resumed a machine that had been paused.
    PausedLease,
    /// This launch resumed a machine that was already ready.
    ReadyLease,
}

impl LaunchPreparation {
    /// Reads the path off a receipt's preparation observation, or nothing when it carries none.
    #[must_use]
    pub const fn of(observation: &soma::Observation<soma::PreparationClass>) -> Option<Self> {
        let soma::Observation::Observed(class) = observation else {
            return None;
        };
        Some(match class {
            soma::PreparationClass::PreparedWorker => Self::Prepared,
            soma::PreparationClass::OnDemand => Self::OnDemand,
            soma::PreparationClass::PausedLease => Self::PausedLease,
            soma::PreparationClass::ReadyLease => Self::ReadyLease,
        })
    }

    /// The name this path is reported under.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::OnDemand => "on_demand",
            Self::PausedLease => "paused_lease",
            Self::ReadyLease => "ready_lease",
        }
    }
}
