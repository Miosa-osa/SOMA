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
    ArtifactDescriptor, ArtifactRole, BuildHost, CompileError, CompileErrorKind, CompileGeneration,
    CompilePhase, CompiledGeneration, CompilerProfile, ContractBinding, ErofsEvidence,
    GenerationManifest, InitramfsContents, LifetimeLimits, MachineInputs, OverlayEvidence,
    PublishedGeneration, Sha256Digest, SnapshotBinding, StartupBehavior, TemplateImage,
    TemplateRevision, ToolOutcome, Toolchain, TreeBounds, VerifiedGeneration, VerifiedKernel,
    VerifiedKernelConfig, compile_generation, contracts, derive_generation_id, erofs, initramfs,
    kernel, kernel_config, manifest as generation_manifest, open_artifact, overlay, template,
    verify_generation, verify_kernel_config,
};
pub use import::import_oci_layout;
pub use normalize::{
    NormalizeError, NormalizeErrorKind, NormalizeOciRootfs, NormalizePhase, NormalizedRootfs,
    RootfsLimits, normalize_oci_rootfs,
};
pub use types::{ImportLimits, ImportOciLayout, ImportedOci, OciSelection};
