//! The network policy class bound into the manifest, and the digest of the policy itself.
//!
//! The class is a coarse statement about a policy that a manifest can compare cheaply, and the
//! digest is the exact policy. They live together because a manifest carries both and nothing
//! else needs either.

use soma::NetworkPolicy;

use crate::generation::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

/// The network policy classes bound into the manifest alongside the policy digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkPolicyClass {
    /// The fail-closed isolated policy.
    Isolated,
    /// Every runtime-controlled dimension deferred to the operator default profile.
    RuntimeDefault,
    /// An explicitly composed policy.
    Explicit,
}

impl NetworkPolicyClass {
    /// Classifies a policy.
    #[must_use]
    pub fn of(policy: &NetworkPolicy) -> Self {
        if *policy == NetworkPolicy::isolated() {
            Self::Isolated
        } else if *policy == NetworkPolicy::runtime_default() {
            Self::RuntimeDefault
        } else {
            Self::Explicit
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Isolated => 0,
            Self::RuntimeDefault => 1,
            Self::Explicit => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Isolated,
            1 => Self::RuntimeDefault,
            2 => Self::Explicit,
            _ => return None,
        })
    }
}

/// Digests the canonical serialization of a network policy.
///
/// The portable `soma` policy serializes with declaration-ordered struct fields and an
/// ordered port set, so the bytes carry no implementation-dependent ordering.
///
/// # Errors
///
/// Returns [`CompileErrorKind::InvalidInput`] when the policy cannot be serialized.
pub fn network_policy_digest(policy: &NetworkPolicy) -> Result<Sha256Digest, CompileError> {
    let bytes = serde_json::to_vec(policy).map_err(|_| {
        CompileError::new(CompilePhase::ResolveInputs, CompileErrorKind::InvalidInput)
    })?;
    Ok(Sha256Digest::of(&bytes))
}
