#![doc = "Verified OCI input artifacts for later SOMA Generation construction."]
#![forbid(unsafe_code)]

mod digest;
mod error;
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
pub use import::import_oci_layout;
pub use normalize::{
    NormalizeError, NormalizeErrorKind, NormalizeOciRootfs, NormalizePhase, NormalizedRootfs,
    RootfsLimits, normalize_oci_rootfs,
};
pub use types::{ImportLimits, ImportOciLayout, ImportedOci, OciSelection};
