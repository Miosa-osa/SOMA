//! Re-verification of published Candidates and Generations, and admission of installed ones.
//!
//! Two different jobs live here and are kept apart. Content verification re-reads a published
//! manifest and every artifact it names, which is the costly boundary an operator runs before
//! publishing an identity. Admission instead trusts an installation that already did that work
//! and rechecks only the small content-addressed manifest, the compiler contract, and the exact
//! shape of the launch handles handed to it.
//!
//! Host-profile compatibility is the third job, and it is the one every path here shares: a
//! decoded manifest arrives from bytes a hostile party may have produced, so `profile` decides
//! what the compiler will accept before anything else looks at it.

mod admission;
mod content;
mod incompatibility;
mod machine;
mod profile;
#[cfg(test)]
mod tests;

pub use admission::{
    InstalledGeneration, admit_installed_generation, admit_verified_handoff,
    declared_policy_version, installed_policy_version,
};
pub use content::{VerifiedCandidate, VerifiedGeneration, verify_candidate, verify_generation};
pub use incompatibility::Incompatibility;

use super::error::{CompileError, CompileErrorKind, CompilePhase};
use profile::require_profile;

const MAX_TREE_MANIFEST_BYTES: u64 = 512 * 1024 * 1024;
const EXT4_MAGIC: u16 = 0xEF53;

fn from_import(error: crate::ImportError) -> CompileError {
    CompileError::from_import(CompilePhase::VerifyGeneration, error)
}

const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::VerifyGeneration, CompileErrorKind::Integrity)
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::VerifyGeneration, CompileErrorKind::Io)
}
