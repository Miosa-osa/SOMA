//! The live half of snapshot format v2: capturing a running `x86_64` machine at its
//! disconnected repair point and restoring it into fresh Instances.
//!
//! The codec, the ordering schedules, and the compatibility rules live in
//! [`crate::snapshot`]; this module is the part that touches KVM. It reads no state before
//! the machine is quiescent, writes no state after the vCPU resumes, and never lets a captured
//! authority, backend, descriptor, or host path cross into a snapshot or out of one.
//!
//! Version 2 captures at one point only: the wait the pinned guest agent enters before any
//! launch page exists. Nothing that identifies an Instance has been created yet, so the
//! memory image cannot carry one.

mod artifacts;
#[cfg(test)]
mod artifacts_tests;
mod capture;
mod device;
mod error;
mod installation;
mod marker;
#[allow(unsafe_code)]
mod platform;
mod profile;
mod quiesce;
mod restore;
#[allow(unsafe_code)]
mod vcpu;

pub use crate::snapshot::inspection::{ArtifactEvidence, CaptureExpectation, VerifiedCapture};
pub use artifacts::SnapshotPaths;
pub use capture::{CaptureOutcome, CaptureRequest, capture};
pub use error::{Artifact, SnapshotError};
pub use installation::inspect as inspect_capture_artifacts;
pub use restore::{
    RestoreFacts, RestoreRequest, Restored, Sterile, SterileRequest, restore, restore_sterile,
};

pub(in crate::x86_64) use platform::write_routing;
