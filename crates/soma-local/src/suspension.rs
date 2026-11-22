//! Suspending a managed Machine into a sealed Suspension, and resuming one as a new Instance
//! (ADR 0034).
//!
//! A suspension reads the running machine into a staging directory, ends the Instance with a
//! `Suspended` receipt, and seals the three members under a fresh data key in this host's store;
//! the plaintext is removed once sealed. A resume decrypts the members, launches a new Instance
//! that restores them instead of its Generation's snapshot, and destroys the Suspension once that
//! Instance runs, so one Suspension resumes at most once.

use std::path::Path;

use soma::{
    ExecutionReceipt, InstanceId, ManagedFailure, OciImage, OperationId, SuspendMachineRequest,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod store;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use store::Suspensions;

/// The file a resume directory names its Generation in, which the launch checks before it
/// restores anything: members captured over one root must never run over another.
pub const RESUME_GENERATION_FILE: &str = "generation";

/// What a caller asks of a suspension.
#[derive(Clone, Debug)]
pub struct SuspendRequest {
    pub operation_id: OperationId,
    pub instance_id: InstanceId,
    /// Who owns the Suspension; it is bound into the sealed header.
    pub tenant: String,
    /// The image the Instance was launched from, kept so a resume need not name it again.
    pub image: Option<OciImage>,
    /// How long the Suspension is kept, capped at the store's ceiling.
    pub retention_secs: u64,
}

/// A sealed Suspension.
#[derive(Clone, Debug)]
pub struct Suspended {
    /// The Suspension's identity, 32 lowercase hexadecimal characters.
    pub suspension_id: String,
    pub expires_unix: u64,
    /// The plaintext bytes sealed: memory, private head and machine state.
    pub sealed_bytes: u64,
    /// The receipt that ended the Instance.
    pub receipt: ExecutionReceipt,
}

/// What a caller asks of a resume.
#[derive(Clone, Debug)]
pub struct ResumeRequest {
    pub operation_id: OperationId,
    /// The new Instance; a resume never reuses the suspended one's identity.
    pub instance_id: InstanceId,
    pub suspension_id: String,
    /// The image to resolve the Generation from, when the suspension did not keep one.
    pub image: Option<OciImage>,
}

/// Why a suspension or a resume did not happen.
#[derive(Debug)]
pub enum SuspensionFailure {
    /// The engine's typed failure; `Backend(Unsupported)` means the Instance keeps running.
    Managed(ManagedFailure),
    /// No Suspension of that identity is held here.
    Unknown,
    /// The Suspension expired and has been destroyed.
    Expired,
    /// The Suspension was taken on a host with another processor profile.
    OtherHost,
    /// Neither the request nor the Suspension names an image to resume from.
    NoImage,
    /// The read started and failed: the Instance has ended, and its receipt says Failed.
    Lost(Box<ExecutionReceipt>),
    /// The Instance was suspended but the store refused the members; they are gone.
    Store(String),
}

impl From<ManagedFailure> for SuspensionFailure {
    fn from(failure: ManagedFailure) -> Self {
        Self::Managed(failure)
    }
}

/// The answer of a runtime that cannot suspend: the Instance keeps running.
#[cfg_attr(all(target_os = "linux", target_arch = "x86_64"), allow(dead_code))]
pub(crate) const fn unsupported() -> SuspensionFailure {
    SuspensionFailure::Managed(ManagedFailure::Backend(
        soma::BackendFailureKind::Unsupported,
    ))
}

/// The engine request that reads one Instance into a staging directory.
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub(crate) fn engine_request(request: &SuspendRequest, staging: &Path) -> SuspendMachineRequest {
    SuspendMachineRequest::new(
        request.operation_id.clone(),
        request.instance_id.clone(),
        staging.to_path_buf(),
    )
}
