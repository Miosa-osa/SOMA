use soma::{
    DestroyMachineRequest, ExecuteMachineRequest, FileMachineRequest, InspectMachineRequest,
    InstanceId, LaunchMachineRequest, MachineListing, MachineRelease, ManagedFailure,
    PtyMachineRequest, StopMachineRequest, TerminalCensus,
};

use crate::tunnel::GuestStream;

mod outcomes;
mod storage;

use outcomes::unsupported_suspension;
pub use outcomes::{
    CommandOutcome, FileOutcome, LifecycleOutcome, SandboxSnapshot, TerminalOutcome,
};
pub use soma_local::{ResizeFailure, ResizeOutcome, ResizeRequest, TunnelFailure};
pub use storage::SandboxStorage;

pub use crate::volumes::{
    OwnedVolume, VolumeBinding, VolumeBody, VolumeFailure, VolumeOperation, VolumeReport,
};

/// The lifecycle surface the HTTP handlers drive.
///
/// The methods take the portable facade's own request types and return its own failure type, so
/// the HTTP layer neither reinterprets a request nor reclassifies a failure on its way through.
/// The trait exists only so the handlers can be exercised without KVM: the production
/// implementation forwards every call straight to `soma_local::LocalRuntime`, which is the same
/// path the CLI takes.
///
/// A method that cannot be implemented honestly is worse here than no method at all, so every
/// method on it is one the engine can actually perform. What a given backend cannot serve is
/// reported as the capability that is missing, not as an invented answer.
pub trait SandboxFacade: SandboxStorage {
    /// Whether a sandbox this facade creates can still be addressed by a later request.
    ///
    /// This service opens one runtime per connection, so a backend that keeps the machine in
    /// the launching process produces an identity nothing can use. Reporting that up front is
    /// what stops `create` returning 201 for a sandbox that is already gone.
    fn hosts_addressable_sandboxes(&self) -> bool;

    /// Creates one durably managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn launch(&mut self, request: LaunchMachineRequest)
    -> Result<LifecycleOutcome, ManagedFailure>;

    /// Looks up the persisted ownership label for restore and exact replay.
    /// # Errors
    /// Returns a storage failure if ownership could not be verified.
    fn restore_owner(
        &mut self,
        _instance: &soma::InstanceId,
    ) -> Result<Option<soma::MachineName>, ManagedFailure> {
        Ok(None)
    }

    /// Restores a stopped sandbox from its own published checkpoint.
    /// # Errors
    /// Returns the managed restore refusal or launch failure.
    fn restore(
        &mut self,
        _request: LaunchMachineRequest,
        _checkpoint: &soma::OperationId,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        Err(ManagedFailure::Backend(
            soma::BackendFailureKind::Unsupported,
        ))
    }

    /// Forks a checkpoint into a separate managed sandbox.
    /// # Errors
    /// Returns a typed refusal if the backend cannot establish the fork.
    fn fork(
        &mut self,
        _request: LaunchMachineRequest,
        _source: &soma::InstanceId,
        _checkpoint: &soma::OperationId,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        Err(ManagedFailure::Backend(
            soma::BackendFailureKind::Unsupported,
        ))
    }

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

    /// Runs one bounded command like [`Self::execute`], reporting each output chunk to `sink`
    /// while the command runs.
    ///
    /// The outcome is the one `execute` returns, so the receipt and the retained output are the
    /// same whichever way the caller asked. The default runs the command to its end and then
    /// replays what it retained, so an implementation that cannot stream still answers; one
    /// that can overrides this.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn execute_streaming(
        &mut self,
        request: ExecuteMachineRequest,
        sink: soma::OutputSink<'_>,
    ) -> Result<CommandOutcome, ManagedFailure> {
        let outcome = self.execute(request)?;
        let output = soma::ObservedOutput::new(
            outcome.stdout.clone(),
            outcome.stdout.len() as u64,
            outcome.stderr.clone(),
            outcome.stderr.len() as u64,
        );
        output.replay(sink);
        Ok(outcome)
    }

    /// Performs one bounded filesystem operation inside a managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure. A cause the guest reported is not a failure:
    /// it arrives in the outcome's answer, because the guest was reached and declined.
    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure>;

    /// Performs one bounded terminal operation inside a managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure. A cause the guest reported is not a failure:
    /// it arrives in the outcome's answer, because the guest was reached and declined.
    fn terminal(&mut self, request: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure>;

    /// Lists the sandboxes this service's durable state holds that have not been released.
    ///
    /// A record the store cannot decode is named in the listing rather than failing it.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn list(&mut self) -> Result<MachineListing, ManagedFailure>;

    /// Counts this service's durable records and names the ones that reached a terminal phase.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn terminal_census(&mut self) -> Result<TerminalCensus, ManagedFailure>;

    /// Releases the durable record of one sandbox that reached a terminal phase, and keeps any
    /// other.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn release(&mut self, instance_id: &InstanceId) -> Result<MachineRelease, ManagedFailure>;

    /// Gracefully stops one managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn stop(&mut self, request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure>;

    /// Reads one running sandbox into a sealed Suspension and ends it (ADR 0034).
    ///
    /// The default refuses as unsupported, which leaves the sandbox running.
    ///
    /// # Errors
    ///
    /// Returns the typed suspension failure.
    fn suspend(
        &mut self,
        _request: &soma_local::SuspendRequest,
    ) -> Result<soma_local::Suspended, soma_local::SuspensionFailure> {
        Err(unsupported_suspension())
    }

    /// Resumes a Suspension as a new sandbox, then destroys the Suspension.
    ///
    /// # Errors
    ///
    /// Returns the typed suspension failure; the Suspension is kept when nothing resumed.
    fn resume_suspension(
        &mut self,
        _request: &soma_local::ResumeRequest,
    ) -> Result<LifecycleOutcome, soma_local::SuspensionFailure> {
        Err(unsupported_suspension())
    }

    /// What this host has left for sandboxes, or `None` for a facade with no host to read.
    fn capacity(&mut self) -> Option<soma_local::capacity::HostCapacity> {
        None
    }

    /// Discards a Suspension without resuming it.
    ///
    /// # Errors
    ///
    /// Returns the typed suspension failure.
    fn discard_suspension(
        &mut self,
        _suspension_id: &str,
    ) -> Result<(), soma_local::SuspensionFailure> {
        Err(unsupported_suspension())
    }

    /// Force-destroys one managed sandbox.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    fn destroy(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure>;

    /// Opens a connected byte stream to `port` on the guest of one hosted sandbox.
    ///
    /// The guest's network is reachable only from inside its namespace, so the stream comes from
    /// the process that holds the sandbox, through the network broker. A backend with no such
    /// path reports [`TunnelFailure::Unavailable`] rather than an invented stream.
    ///
    /// # Errors
    ///
    /// Returns the typed tunnel failure.
    fn tunnel(
        &mut self,
        _instance_id: &InstanceId,
        _port: u16,
    ) -> Result<GuestStream, TunnelFailure> {
        Err(TunnelFailure::Unavailable)
    }

    /// Pauses guest execution and keeps the sandbox resident.
    ///
    /// # Errors
    ///
    /// Returns the owner's failure when no pause can be confirmed.
    fn pause(&mut self, _instance_id: &InstanceId) -> Result<(), soma_local::PauseFailure> {
        Err(soma_local::PauseFailure::Unavailable)
    }

    /// Resumes the exact hosted machine after the owning runtime confirms it.
    ///
    /// # Errors
    ///
    /// Returns the owner's failure when no resume can be confirmed.
    fn resume(&mut self, _instance_id: &InstanceId) -> Result<(), soma_local::ResumeFailure> {
        Err(soma_local::ResumeFailure::Unavailable)
    }

    /// Changes the CPU and memory of one hosted sandbox live, and reports what is now true.
    /// An empty request changes nothing and reports the current values.
    ///
    /// # Errors
    ///
    /// Returns the typed resize failure; a machine that cannot resize live is an outcome with
    /// `Unsupported` dimensions, not a failure.
    fn resize(
        &mut self,
        _instance_id: &InstanceId,
        _request: ResizeRequest,
    ) -> Result<ResizeOutcome, ResizeFailure> {
        Err(ResizeFailure::Unavailable)
    }

    /// Opens or closes the egress of a sandbox that was launched windowed, and reports what is
    /// now in force. Anything other than an outcome means the egress may not be as asked.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; a backend that holds no bundle to change is `Unavailable`.
    fn set_egress(
        &mut self,
        _instance_id: &InstanceId,
        _request: soma_local::EgressSetRequest,
    ) -> Result<soma_local::EgressSetOutcome, soma_local::EgressSetFailure> {
        Err(soma_local::EgressSetFailure::Unavailable)
    }
}
