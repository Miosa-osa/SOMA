//! Projecting a validated Template Lock onto the Generation compiler's input contract.

use std::{error::Error, fmt};

use soma::OciPlatform;
use soma_template::{
    BackendCapabilities, IdleAction, ResourceLimits, RevisionError,
    TemplateRevision as LockRevision,
};

use crate::{CompileError, LifetimeLimits, StartupBehavior, TemplateImage, TemplateRevision};

/// Why a Template Lock could not become a compiler revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockProjectionError {
    /// The revision view carries no image reference, so no provenance was attached to it.
    MissingReference,
    /// The lock states something the portable request contract cannot express.
    Unrepresentable(RevisionError),
    /// The lock states something the compiler profile does not accept.
    Unsupported(CompileError),
}

impl fmt::Display for LockProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingReference => {
                formatter.write_str("the revision view has no image reference attached")
            }
            Self::Unrepresentable(error) => write!(formatter, "{error}"),
            Self::Unsupported(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for LockProjectionError {}

/// Builds the compiler's Template revision from the `soma-template` revision view of a Lock.
///
/// The view must carry provenance, because the compiler binds the authored image reference
/// alongside the resolved digest and a Lock deliberately excludes the mutable reference text.
/// Every locked field this drops is listed in the [module documentation](super).
///
/// # Errors
///
/// Returns [`LockProjectionError::MissingReference`] for a view without provenance,
/// [`LockProjectionError::Unrepresentable`] when the locked network envelope has no portable
/// policy, and [`LockProjectionError::Unsupported`] when the locked shape or lifetime falls
/// outside the compiler profile.
pub fn compiler_revision(
    view: &LockRevision,
    profile_version: u16,
) -> Result<TemplateRevision, LockProjectionError> {
    let image = view.image();
    let reference = image
        .reference()
        .ok_or(LockProjectionError::MissingReference)?;
    let shape = view.shape().map_err(LockProjectionError::Unrepresentable)?;
    let lifetime =
        LifetimeLimits::new(view.ttl_seconds()).map_err(LockProjectionError::Unsupported)?;
    TemplateRevision::new(
        TemplateImage::new(
            reference.clone(),
            image.manifest_digest().clone(),
            image.platform().clone(),
        ),
        shape,
        // Readiness only: a module health probe is the natural source of an explicit workload
        // probe, and no locked module reaches the compiler yet.
        StartupBehavior::readiness_only(),
        lifetime,
        profile_version,
    )
    .map_err(LockProjectionError::Unsupported)
}

/// The Backend capabilities of the compiler's `x86_64` profile version 1.
///
/// Validation intersects these with the Template, so a document the compiler would later refuse
/// is rejected while the offending field can still be named. The vCPU, memory, and storage
/// bounds are profile v1's own; the storage ceiling is this profile's largest overlay template,
/// which the compiler does not bound itself.
///
/// # Panics
///
/// Cannot panic: the platform list is one entry and every bound is a constant.
#[must_use]
pub fn profile_v1_backend() -> BackendCapabilities {
    BackendCapabilities::new(
        &[OciPlatform::linux_amd64()],
        // Stopping or checkpointing an idle Instance is a Backend lifecycle the KVM adapter
        // does not offer through a prepared Generation yet, so destroy is the only action.
        &[IdleAction::Destroy],
        ResourceLimits {
            max_vcpus: 1,
            max_memory_mib: 3 * 1024,
            max_writable_storage_mib: 64 * 1024,
        },
    )
    .expect("one platform is a bounded, non-empty capability list")
}
