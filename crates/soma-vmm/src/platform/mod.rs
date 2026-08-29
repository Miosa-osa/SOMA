mod interface;
mod outcome;
mod progress;
mod readiness;
mod restore;
mod unavailable;

pub(super) use interface::Platform;
pub(super) use outcome::{PlatformExecution, PlatformFailure, PlatformStop};
pub(super) use progress::{ReadinessProgress, RestoreProgress};
#[cfg(test)]
pub(super) use progress::{ReadinessStep, RestoreStep};
pub(super) use readiness::{ReadinessFailure, ReadinessFailurePoint, ReadyAuthenticatedGuest};
pub(super) use restore::{RestoreFailure, RestoreFailurePoint, RestoredMachine};
pub(super) use unavailable::UnavailablePlatform;
