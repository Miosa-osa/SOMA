//! Opening and closing the egress of a windowed sandbox, and saying exactly what is now true.
//!
//! Only a sandbox created with a windowed network holds a broker bundle whose rules can be
//! changed. Every answer states the egress now in force, so a caller never has to infer it from
//! having asked.

use serde::{Deserialize, Serialize};

/// What a caller wants the sandbox's egress to be.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EgressSetRequest {
    /// `true` allows the public internet; `false` denies everything.
    pub open: bool,
    /// Whether the broker's resolvers are reachable. Refused with `open: false`.
    pub dns: bool,
}

/// The egress in force once a request was answered.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EgressSetOutcome {
    /// Whether guest egress is now permitted.
    pub open: bool,
    /// Whether the resolvers are now reachable.
    pub dns: bool,
    /// Whether the rules differed from before the call.
    pub changed: bool,
}

/// Why the egress was not changed, or may not have been.
///
/// A caller that asked to close and got anything but an outcome must treat the sandbox as
/// possibly open.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EgressSetFailure {
    /// No such sandbox is hosted here.
    NotFound,
    /// The sandbox was not created with a windowed network, so there is nothing to change.
    NotWindowed,
    /// The sandbox's bundle is not activated yet (or any more).
    NotActive,
    /// The broker could not apply the rules. After a failed open the sandbox is still denied;
    /// after a failed close it may not be.
    FlipFailed,
    /// The host or the broker could not be reached.
    Unavailable,
}

/// Asks the process hosting a sandbox to change its egress.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn set_egress_machine(
    directory: &std::path::Path,
    instance: &soma::InstanceId,
    request: EgressSetRequest,
) -> Result<EgressSetOutcome, EgressSetFailure> {
    crate::backend::set_egress_machine(directory, instance, request)
}

/// Only the KVM backend holds a bundle to change.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn set_egress_machine(
    _directory: &std::path::Path,
    _instance: &soma::InstanceId,
    _request: EgressSetRequest,
) -> Result<EgressSetOutcome, EgressSetFailure> {
    Err(EgressSetFailure::Unavailable)
}
