#![doc = "Portable use-case orchestration and execution evidence for SOMA."]
#![forbid(unsafe_code)]

mod backend;
mod engine;
mod evidence;
mod fingerprint;
mod identity;
mod machine_name;
mod output;
mod receipt;
mod request;
mod state_store;

pub use backend::{
    Backend, BackendFailure, BackendFailureKind, CleanupObservation, CleanupReason, CleanupRequest,
    CleanupTimes, CommandObservation, CommandTimes, ExecutionRequest, InspectionObservation,
    InspectionRequest, LaunchObservation, LaunchRequest, LaunchTimes, ResolutionObservation,
    ResolutionRequest,
};
pub use engine::{
    DestroyMachineRequest, Engine, ExecuteMachineRequest, FailurePhase, InspectMachineRequest,
    LaunchMachineRequest, MachineDestroy, MachineExecution, MachineInspection, MachineLaunch,
    MachineStop, ManagedFailure, ManagedStateError, ReplayEvidence, RunFailure, RunFailureKind,
    RunOutcome, StopMachineRequest,
};
pub use evidence::{
    AssignedAddress, BackendKind, CleanupDisposition, CleanupEvidence, CleanupMethod,
    CommandStatus, DigestBinding, EffectiveNetwork, EffectivePortPublication, EffectiveShape,
    EvidenceClass, IsolationClass, MAX_ASSIGNED_ADDRESSES, MachineState, MeasurementBoundary,
    MeasurementClass, MeasurementClock, MeasurementOrigin, MeasurementTerminal, Milestone,
    MilestoneKind, NetworkAttachment, NetworkCleanupEvidence, Observation, ObservationUnavailable,
    OperationKind, PortActivationClass, PreparationClass, TerminalStatus, WorkloadEvidence,
    WorkloadIdentity,
};
pub use identity::{GenerationId, InstanceId, OperationId, RequestFingerprint};
pub use machine_name::MachineName;
pub use output::{CapturedOutput, ObservedOutput, OutputMetadata, StreamMetadata};
pub use receipt::{ExecutionReceipt, ReceiptValidationError};
pub use request::{
    Capabilities, DirectCommand, DnsPolicy, EgressPolicy, ExecutionLimits, GuestAddressIntent,
    HostBind, HostPort, Ipv4AddressIntent, Ipv6AddressIntent, MAX_DNS_SERVERS,
    MAX_PORT_PUBLICATIONS, MachineShape, NetworkPolicy, NetworkProfileId, NetworkProfileSelector,
    OciDigest, OciImage, OciPlatform, PortPublication, ProfileRevision, ProxyPolicy,
    ProxyProfileId, ProxyProfileSelector, RunRequest, TransportProtocol, ValidationError,
};
pub use state_store::{
    MAX_STATE_RECORD_BYTES, MemoryStateStore, StateRecord, StateRevision, StateStore,
    StateStoreFailure, StateStoreFailureKind, StoredState,
};
