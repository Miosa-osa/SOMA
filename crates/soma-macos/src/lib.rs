#![deny(unsafe_code)]
#![deny(warnings)]

//! Development-only macOS sandbox execution through Apple's `container` CLI.
//!
//! This crate deliberately does not certify the production Linux KVM path.
//! It supplies a local OCI compatibility adapter backed by one Virtualization.framework virtual
//! machine per container.

mod backend;
mod error;
mod image;
mod process;
mod request;
mod result;

pub use backend::MacOsBackend;
pub use error::{
    BackendError, CommandFailure, CommandFailureReason, ImageResolutionFailure, Operation,
    OwnershipFailure, ProcessFailureKind,
};
pub use image::{
    ContentDigest, ImageBinding, ImagePlatform, ImageResolutionTimings, ImageSourceReference,
    ResolvedImage,
};
pub use request::{
    ControlLimits, CreateMachine, DnsConfiguration, ExecuteCommand, ExecutionLimits, GuestCommand,
    ImageReference, InstanceId, MachineShape, NetworkConfiguration, NetworkPolicy, OneShotRun,
    PublishedPort, RequestError, RequestErrorReason, StopOptions, TransportProtocol,
};
pub use result::{
    BackendClass, CapabilityReport, CleanupState, ComponentVersion, ControlReceipt, CreatedMachine,
    ExecutionResult, ExecutionStatus, InspectedMachine, InspectedNetwork, IsolationKind,
    MachineResources, NetworkAddress, NetworkAttachment,
};

/// The fail-closed Apple `container` CLI range whose command contract this adapter accepts.
pub const SUPPORTED_CONTAINER_VERSION_REQUIREMENT: &str = ">=1.3.0,<1.4.0";

#[cfg(test)]
mod tests;
