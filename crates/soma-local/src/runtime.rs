use soma::{
    Backend as _, BackendKind, DestroyMachineRequest, Engine, ExecuteMachineRequest,
    FileMachineRequest, LaunchMachineRequest, MachineDestroy, MachineExecution, MachineFile,
    MachineInspection, MachineLaunch, MachinePty, MachineStop, ManagedFailure, PtyMachineRequest,
    RunFailure, RunOutcome, RunRequest, SandboxEntry, StopMachineRequest,
};

use crate::{
    BackendSelection, FileStateStore, LocalFailure, LocalFailureKind, LocalRuntimeConfig,
    backend::LocalBackend,
};

pub struct LocalRuntime {
    engine: Engine<LocalBackend, FileStateStore>,
    state_root: std::path::PathBuf,
    host_directory: Option<std::path::PathBuf>,
    configured_backend: BackendSelection,
    backend_kind: BackendKind,
}

impl LocalRuntime {
    /// Opens the selected fail-closed local backend and shared durable state store.
    ///
    /// # Errors
    ///
    /// Returns a typed local setup failure when target selection, runtime probing, or state-store
    /// setup cannot be completed safely.
    pub fn open(config: LocalRuntimeConfig) -> Result<Self, LocalFailure> {
        let host_directory = config
            .hosted_machines
            .then(|| crate::backend::machine_host_directory(&config.state_root));
        let (backend, configured_backend) =
            LocalBackend::open(config.backend, config.runtime, host_directory.clone())?;
        let backend_kind = backend.kind();
        let state_root = config.state_root.clone();
        let state = FileStateStore::open(config.state_root)
            .map_err(|failure| LocalFailure::new(LocalFailureKind::StateStore(failure.kind())))?;
        Ok(Self {
            engine: Engine::with_state_store(backend, state),
            state_root,
            host_directory,
            configured_backend,
            backend_kind,
        })
    }

    #[must_use]
    pub const fn configured_backend(&self) -> BackendSelection {
        self.configured_backend
    }

    #[must_use]
    pub const fn backend_kind(&self) -> BackendKind {
        self.backend_kind
    }

    /// Whether this runtime can give a Machine the network `policy` describes.
    ///
    /// A configuration whose machines would need a network the host cannot give is refused when
    /// it is loaded, rather than by every launch (SOMA#56).
    #[must_use]
    pub fn serves_network(&self, policy: &soma::NetworkPolicy) -> bool {
        self.engine.backend().serves_network(policy)
    }

    /// Whether a Machine this runtime launches outlives the process that launched it.
    #[must_use]
    pub const fn machine_hosting(&self) -> crate::MachineHosting {
        crate::machine_hosting(self.backend_kind)
    }

    /// Runs one facade-owned, cleanup-proven local transaction.
    ///
    /// # Errors
    ///
    /// Returns the facade's evidence-carrying run failure.
    pub fn run(&mut self, request: RunRequest) -> Result<RunOutcome, RunFailure> {
        self.engine.run(request)
    }

    /// Launches one durably managed local Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn launch_machine(
        &mut self,
        request: LaunchMachineRequest,
    ) -> Result<MachineLaunch, ManagedFailure> {
        self.engine.launch_machine(request)
    }

    /// Reads the persisted ownership label for a possibly recoverable Instance.
    /// # Errors
    /// Returns the durable-store failure if ownership cannot be read.
    pub fn restore_owner(
        &mut self,
        instance: &soma::InstanceId,
    ) -> Result<Option<soma::MachineName>, ManagedFailure> {
        self.engine.restore_owner(instance)
    }

    /// Restores an existing stopped Instance from a published checkpoint.
    /// # Errors
    /// Returns the typed managed failure if restore could not be admitted or completed.
    pub fn restore_machine(
        &mut self,
        request: LaunchMachineRequest,
        checkpoint: &soma::OperationId,
    ) -> Result<MachineLaunch, ManagedFailure> {
        self.engine.restore_machine(request, checkpoint)
    }

    /// Forks a checkpoint into a distinct durably managed Machine with the same owner.
    /// # Errors
    /// Returns the typed managed failure if the fork could not be admitted or completed.
    pub fn fork_machine(
        &mut self,
        request: LaunchMachineRequest,
        source: &soma::InstanceId,
        checkpoint: &soma::OperationId,
    ) -> Result<MachineLaunch, ManagedFailure> {
        self.engine.fork_machine(request, source, checkpoint)
    }

    /// Executes one bounded command in a durably managed Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn execute_machine(
        &mut self,
        request: ExecuteMachineRequest,
    ) -> Result<MachineExecution, ManagedFailure> {
        self.engine.execute_machine(request)
    }

    /// Executes one command like [`Self::execute_machine`] and reports each output chunk to
    /// `sink` as the backend observes it.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn execute_machine_streaming(
        &mut self,
        request: ExecuteMachineRequest,
        sink: soma::OutputSink<'_>,
    ) -> Result<MachineExecution, ManagedFailure> {
        self.engine.execute_machine_streaming(request, sink)
    }

    /// Performs one bounded filesystem operation in a durably managed Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn file_machine(
        &mut self,
        request: FileMachineRequest,
    ) -> Result<MachineFile, ManagedFailure> {
        self.engine.file_machine(request)
    }

    /// Performs one bounded terminal operation in a durably managed Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn pty_machine(
        &mut self,
        request: PtyMachineRequest,
    ) -> Result<MachinePty, ManagedFailure> {
        self.engine.pty_machine(request)
    }

    /// Lists the sandboxes this runtime's durable state holds, with what the backend can reach.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn list_machines(&mut self) -> Result<Vec<SandboxEntry>, ManagedFailure> {
        self.engine.list_machines()
    }

    /// Lists the sandboxes like [`Self::list_machines`], naming a record the store cannot decode
    /// instead of failing the listing.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure when the store cannot be read at all.
    pub fn list_machines_reporting_unreadable(
        &mut self,
    ) -> Result<soma::MachineListing, ManagedFailure> {
        self.engine.list_machines_reporting_unreadable()
    }

    /// Inspects one durably managed Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn inspect_machine(
        &mut self,
        request: soma::InspectMachineRequest,
    ) -> Result<MachineInspection, ManagedFailure> {
        self.engine.inspect_machine(request)
    }

    /// Opens a byte stream to `port` on the guest of a hosted Machine.
    ///
    /// The stream is already connected: bytes written reach the guest's service and the guest's
    /// replies are read back. Closing it closes the connection to the guest. The port must be
    /// one the broker was configured to allow.
    ///
    /// # Errors
    ///
    /// Returns the typed [`crate::TunnelFailure`].
    #[cfg(unix)]
    pub fn tunnel_machine(
        &self,
        instance: &soma::InstanceId,
        port: u16,
    ) -> Result<std::os::unix::net::UnixStream, crate::TunnelFailure> {
        let directory = self
            .host_directory
            .as_deref()
            .ok_or(crate::TunnelFailure::Unavailable)?;
        crate::tunnel::tunnel_machine(directory, instance, port)
    }

    /// Changes the CPU and memory of a hosted Machine live, and reports what actually changed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ResizeFailure`] when the Machine cannot be reached.
    pub fn resize_machine(
        &self,
        instance: &soma::InstanceId,
        request: crate::ResizeRequest,
    ) -> Result<crate::ResizeOutcome, crate::ResizeFailure> {
        let directory = self
            .host_directory
            .as_deref()
            .ok_or(crate::ResizeFailure::Unavailable)?;
        crate::resize::resize_machine(directory, instance, request)
    }

    /// Captures a running hosted Machine through its owning process.
    ///
    /// Repeating the same operation id returns the existing durable receipt.
    ///
    /// # Errors
    ///
    /// Returns the owner, capture, or publication failure.
    pub fn checkpoint_machine(
        &self,
        instance: &soma::InstanceId,
        operation: &soma::OperationId,
    ) -> Result<crate::CheckpointReceipt, crate::CheckpointFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let directory = self
                .host_directory
                .as_deref()
                .ok_or(crate::CheckpointFailure::Unsupported)?;
            crate::backend::checkpoint_machine(directory, instance, operation)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (instance, operation);
            Err(crate::CheckpointFailure::Unsupported)
        }
    }

    /// Releases a published checkpoint by its exact sandbox, operation and state digest.
    ///
    /// # Errors
    ///
    /// Refuses unconfigured storage, an active restore, a mismatched receipt, or an
    /// unconfirmed storage operation.
    pub fn discard_checkpoint(
        &self,
        instance: &soma::InstanceId,
        checkpoint: &soma::OperationId,
        state_digest: &soma::GenerationId,
    ) -> Result<(), crate::CheckpointFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let state_root = self
                .host_directory
                .as_deref()
                .and_then(std::path::Path::parent)
                .ok_or(crate::CheckpointFailure::Unsupported)?;
            let _lease = crate::file_store::restore_lease(state_root, instance)
                .map_err(|_| crate::CheckpointFailure::Unconfirmed)?;
            crate::backend::discard_checkpoint(instance, checkpoint, state_digest)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (instance, checkpoint, state_digest);
            Err(crate::CheckpointFailure::Unsupported)
        }
    }

    /// Pauses a hosted Machine after receiving confirmation from its owner.
    ///
    /// # Errors
    ///
    /// Returns the typed failure when the owner cannot confirm pause.
    pub fn pause_machine(&self, instance: &soma::InstanceId) -> Result<(), crate::PauseFailure> {
        let directory = self
            .host_directory
            .as_deref()
            .ok_or(crate::PauseFailure::Unavailable)?;
        crate::pause::pause_machine(directory, instance)
    }

    /// Resumes a hosted Machine after receiving confirmation from its owner.
    ///
    /// # Errors
    ///
    /// Returns the typed failure when the owner cannot confirm resume.
    pub fn resume_machine(&self, instance: &soma::InstanceId) -> Result<(), crate::ResumeFailure> {
        let directory = self
            .host_directory
            .as_deref()
            .ok_or(crate::ResumeFailure::Unavailable)?;
        crate::pause::resume_machine(directory, instance)
    }

    /// Opens or closes the egress of a hosted Machine that was launched windowed.
    ///
    /// # Errors
    ///
    /// Returns [`crate::EgressSetFailure`]; anything other than an outcome means the egress may
    /// not be what was asked for.
    pub fn set_egress_machine(
        &self,
        instance: &soma::InstanceId,
        request: crate::EgressSetRequest,
    ) -> Result<crate::EgressSetOutcome, crate::EgressSetFailure> {
        let directory = self
            .host_directory
            .as_deref()
            .ok_or(crate::EgressSetFailure::Unavailable)?;
        crate::egress::set_egress_machine(directory, instance, request)
    }

    /// Gracefully attempts to stop and release one managed Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn stop_machine(
        &mut self,
        request: StopMachineRequest,
    ) -> Result<MachineStop, ManagedFailure> {
        self.engine.stop_machine(request)
    }

    /// Force-destroys and releases one managed Machine.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn destroy_machine(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<MachineDestroy, ManagedFailure> {
        self.engine.destroy_machine(request)
    }

    /// Counts the durable records and names the ones that reached a terminal phase.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure when the store cannot be read.
    pub fn terminal_census(&mut self) -> Result<soma::TerminalCensus, ManagedFailure> {
        self.engine.terminal_census()
    }

    /// Releases the durable record of one Machine that reached a terminal phase.
    ///
    /// # Errors
    ///
    /// Returns the facade's typed managed failure.
    pub fn release_machine(
        &mut self,
        instance: &soma::InstanceId,
    ) -> Result<soma::MachineRelease, ManagedFailure> {
        self.engine.release_machine(instance)
    }
}

impl std::fmt::Debug for LocalRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalRuntime")
            .field("configured_backend", &self.configured_backend)
            .field("backend_kind", &self.backend_kind)
            .finish_non_exhaustive()
    }
}

mod suspension;
