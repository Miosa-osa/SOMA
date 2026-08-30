//! The `x86_64` Generation compiler: canonical tree to immutable machine artifacts.
//!
//! Immutable artifact compilation, captured-snapshot installation, certification, promotion,
//! and ready Generation verification are implemented here.
//! Live guest boot and capture are supplied by the Linux KVM machine layer.

mod artifacts;
mod candidate;
/// The gate between a Candidate and a certified, ready Generation.
pub mod certify;
mod compile;
/// Canonical contract statements, their digests, and the fixed kernel command line.
pub mod contracts;
/// Pinned EROFS formatter invocation, UUID derivation, and retained evidence.
pub mod erofs;
mod erofs_reader;
mod erofs_verify;
mod error;
mod identity;
/// Deterministic `newc` initramfs construction and allowlist verification.
pub mod initramfs;
/// ELF and PVH kernel verification.
pub mod kernel;
/// Kernel configuration facility requirements for profile v1.
pub mod kernel_config;
/// Canonical `SOMAGEN` v2 manifest types, encoder, and decoder.
pub mod manifest;
/// Sterile ext4 overlay-template contract, creation, and verification.
pub mod overlay;
mod process;
mod publish;
mod request;
mod snapshot;
mod tar_stream;
/// Template revision inputs: image, Machine shape, startup, lifetime, and profile version.
pub mod template;
/// The sealed builder environment: every external tool one build executed.
pub mod toolchain;
mod tree_decoder;
mod verify;

pub use artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest};
pub use candidate::{CandidateId, Certification, PublishedCandidate, PublishedGeneration};
pub use certify::{certify_candidate, promote_candidate};
pub use compile::{CompiledCandidate, compile_generation};
pub use contracts::ContractBinding;
pub use erofs::ErofsEvidence;
pub use error::{CompileError, CompileErrorKind, CompilePhase};
pub use identity::derive_generation_id;
pub use initramfs::InitramfsContents;
pub use kernel::VerifiedKernel;
pub use kernel_config::{VerifiedKernelConfig, verify_kernel_config};
pub use manifest::{GenerationManifest, SnapshotBinding};
pub use overlay::OverlayEvidence;
pub use process::ToolOutcome;
pub use publish::open_artifact;
pub use request::{BuildHost, CompileGeneration, CompilerProfile, MachineInputs, Toolchain};
pub use snapshot::{SnapshotSource, install_snapshot};
pub use template::{LifetimeLimits, StartupBehavior, TemplateImage, TemplateRevision};
pub use toolchain::{BoundTool, BuilderEnvironment};
pub use tree_decoder::TreeBounds;
pub use verify::{
    Incompatibility, VerifiedCandidate, VerifiedGeneration, verify_candidate, verify_generation,
};
