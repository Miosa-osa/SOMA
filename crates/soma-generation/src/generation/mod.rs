//! The `x86_64` Generation compiler: canonical tree to immutable machine artifacts.
//!
//! Phases 1 through 3 and 6 of the Generation compiler design are implemented here.
//! Phases 4 and 5, guest boot, snapshot capture, and certification, have no implementation and
//! are represented only as typed absent state in the manifest and result types.

mod artifacts;
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
/// Canonical `SOMAGEN` v1 manifest types, encoder, and decoder.
pub mod manifest;
/// Sterile ext4 overlay-template contract, creation, and verification.
pub mod overlay;
mod process;
mod publish;
mod request;
mod tar_stream;
/// Template revision inputs: image, Machine shape, startup, lifetime, and profile version.
pub mod template;
mod tree_decoder;
mod verify;

pub use artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest};
pub use compile::{CompiledGeneration, compile_generation};
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
pub use publish::PublishedGeneration;
pub use request::{BuildHost, CompileGeneration, CompilerProfile, MachineInputs, Toolchain};
pub use template::{LifetimeLimits, StartupBehavior, TemplateImage, TemplateRevision};
pub use tree_decoder::TreeBounds;
pub use verify::{VerifiedGeneration, verify_generation};
