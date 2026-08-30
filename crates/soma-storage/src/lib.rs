//! XFS reflink storage profile for SOMA private writable disk heads.
//!
//! The immutable EROFS root of a Generation is shared read-only.
//! Writable state lives in one private ext4 overlay head per Instance that is created with
//! `FICLONE` from a sterile size-class template on XFS with `reflink=1`.
//! Launch never formats, grows, scans, or copies a filesystem; the only launch-time work is one
//! clone of a verified template under a capability-owned directory descriptor.
//!
//! The crate is split by policy and mechanism:
//!
//! - [`profile`] owns the published overlay classes, exact-class admission, and the storage
//!   profile that proves XFS with reflink support before any head is created.
//! - [`template`] creates sterile ext4 templates with a pinned `mke2fs` invocation and records
//!   their digest.
//! - [`clone`] creates one private head from a template with `FICLONE`, syncs publication, and
//!   verifies apparent size and extent sharing before handing back an open descriptor only.
//! - [`verify`] is the conformance proof that two clones of one template diverge without
//!   touching the template or each other.
//! - [`lease`], [`release`], and [`reconcile`] own single-use head ownership, destruction, and
//!   the audit of a head directory against the ownership ledger.
//! - [`bench`] is the retained measurement matrix behind the on-demand versus prepared-head
//!   decision in `docs/evidence`.
//!
//! Portable types compile everywhere.
//! Every kernel mechanism is Linux-only and fails closed elsewhere.

#![deny(unsafe_code)]

pub mod head;
pub mod lease;
pub mod profile;
pub mod template;

#[cfg(unix)]
pub mod reconcile;
#[cfg(unix)]
pub mod release;

#[cfg(target_os = "linux")]
pub mod bench;
#[cfg(target_os = "linux")]
pub mod clone;
#[cfg(target_os = "linux")]
pub mod fiemap;
#[cfg(target_os = "linux")]
pub mod verify;

pub use head::{HeadName, HeadNameError, HeadToken, HeadTokenError};
pub use lease::{HeadLedger, LeaseError, LeaseReceipt};
pub use profile::{
    AdmissionRejection, BlockSize, ClassCatalog, ClassName, ClassRejection, Ext4FeatureSet,
    FreeSpaceEvidence, InodePolicy, LogicalBytes, MountOption, MountOptions, OverlayClass,
    OverlayRecipe, ProfileRejection, StorageProfile, TemplateDigest, UuidPolicy,
};
pub use template::{SterileTemplate, TemplateError};

#[cfg(unix)]
pub use reconcile::{Disposition, ReconcileReport};
#[cfg(unix)]
pub use release::{ReleaseError, ReleaseOutcome};

#[cfg(target_os = "linux")]
pub use clone::{CloneError, ClonePhases, ClonedHead, clone_head, clone_head_timed};
#[cfg(target_os = "linux")]
pub use fiemap::ExtentSummary;
