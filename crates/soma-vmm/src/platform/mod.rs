#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod kvm;

mod interface;
mod outcome;
mod progress;
mod readiness;
mod restore;
mod unavailable;

pub(super) use interface::Platform;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) use kvm::KvmPlatform;
pub(super) use outcome::{PlatformExecution, PlatformFailure, PlatformStop};
pub(super) use progress::{ReadinessProgress, RestoreProgress};
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(super) use progress::{ReadinessStep, RestoreStep};
pub(super) use readiness::{ReadinessFailure, ReadinessFailurePoint, ReadyAuthenticatedGuest};
pub(super) use restore::{RestoreFailure, RestoreFailurePoint, RestoredMachine};
pub(super) use unavailable::UnavailablePlatform;
