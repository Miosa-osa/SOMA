use soma::{
    BackendKind, DestroyMachineRequest, ExecuteMachineRequest, ExecutionReceipt, FileAnswer,
    FileMachineRequest, InspectMachineRequest, InstanceId, LaunchMachineRequest, MachineState,
    ManagedFailure, StopMachineRequest, TerminalStatus,
};

/// One managed lifecycle transition, carrying the receipt that proves it happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleOutcome {
    pub instance_id: InstanceId,
    pub receipt: ExecutionReceipt,
}

/// One command execution and the guest output it produced.
///
/// Output is carried as raw bytes rather than a string because guest output is not required to be
/// UTF-8, and the wire layer base64 encodes it exactly as the CLI does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutcome {
    pub instance_id: InstanceId,
    pub status: TerminalStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub receipt: ExecutionReceipt,
}

/// One completed filesystem operation and what the guest answered.
///
/// There is no receipt here because the operation mints none: it runs no program and leaves
/// nothing a later caller could replay, so a receipt would have to be invented to carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileOutcome {
    pub instance_id: InstanceId,
    pub operation: &'static str,
    pub answer: FileAnswer,
}

/// The observed state of one managed sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSnapshot {
    pub instance_id: InstanceId,
    pub state: MachineState,
    pub backend: BackendKind,
    pub receipt: ExecutionReceipt,
}

/// The lifecycle surface the HTTP handlers drive.
///
/// The methods take the portable facade's own request types and return its own failure type, so
/// the HTTP layer neither reinterprets a request nor reclassifies a failure on its way through.
/// The trait exists only so the handlers can be exercised without KVM: the production
/// implementation forwards every call straight to `soma_local::LocalRuntime`, which is the same
/// path the CLI takes.
///
/// It is deliberately narrower than the provider contract. Enumeration is absent because the
/// engine cannot perform it, and a method that cannot be implemented honestly is worse here than
/// no method at all.
pub trait SandboxFacade {
    /// Creates one durably managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn launch(&mut self, request: LaunchMachineRequest)
    -> Result<LifecycleOutcome, ManagedFailure>;

    /// Reads back one managed sandbox by exact instance id.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn inspect(
        &mut self,
        request: InspectMachineRequest,
    ) -> Result<SandboxSnapshot, ManagedFailure>;

    /// Runs one bounded command inside a managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn execute(&mut self, request: ExecuteMachineRequest)
    -> Result<CommandOutcome, ManagedFailure>;

    /// Performs one bounded filesystem operation inside a managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure. A cause the guest reported is not a failure:
    /// it arrives in the outcome's answer, because the guest was reached and declined.
    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure>;

    /// Gracefully stops one managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn stop(&mut self, request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure>;

    /// Force-destroys one managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn destroy(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure>;
}
