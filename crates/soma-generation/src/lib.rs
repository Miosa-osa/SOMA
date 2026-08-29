#![doc = "Verified OCI input, normalized rootfs, and Generation compiler artifacts for SOMA."]
#![deny(unsafe_code)]

mod digest;
mod error;
mod generation;
mod import;
mod layer_tar;
mod layout;
mod manifest;
mod normalize;
mod oci;
mod publish;
mod root;
mod store;
mod tar_preflight;
mod traversal;
mod types;
mod verify;

pub use error::{ImportError, ImportErrorKind, ImportPhase};
pub use generation::{
    ArtifactDescriptor, ArtifactRole, BuildHost, CandidateId, Certification, CompileError,
    CompileErrorKind, CompileGeneration, CompilePhase, CompiledCandidate, CompilerProfile,
    ContractBinding, ErofsEvidence, GenerationManifest, InitramfsContents, LifetimeLimits,
    MachineInputs, OverlayEvidence, PublishedCandidate, PublishedGeneration, Sha256Digest,
    SnapshotBinding, StartupBehavior, TemplateImage, TemplateRevision, ToolOutcome, Toolchain,
    TreeBounds, VerifiedCandidate, VerifiedGeneration, VerifiedKernel, VerifiedKernelConfig,
    certify, certify_candidate, compile_generation, contracts, derive_generation_id, erofs,
    initramfs, kernel, kernel_config, manifest as generation_manifest, open_artifact, overlay,
    promote_candidate, template, verify_candidate, verify_generation, verify_kernel_config,
};
pub use import::import_oci_layout;
pub use normalize::{
    NormalizeError, NormalizeErrorKind, NormalizeOciRootfs, NormalizePhase, NormalizedRootfs,
    RootfsLimits, normalize_oci_rootfs,
};
pub use types::{ImportLimits, ImportOciLayout, ImportedOci, OciSelection};
