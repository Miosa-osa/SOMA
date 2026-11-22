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
    suspensions: crate::suspension::Suspensions,
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
        let suspensions = crate::suspension::Suspensions::at(&config.state_root);
        let state = FileStateStore::open(config.state_root)
            .map_err(|failure| LocalFailure::new(LocalFailureKind::StateStore(failure.kind())))?;
        Ok(Self {
            engine: Engine::with_state_store(backend, state),
            suspensions,
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

    /// Reads one managed Machine into a sealed Suspension and ends it (ADR 0034).
    ///
    /// # Errors
    ///
    /// Returns [`crate::SuspensionFailure::Managed`] with `Backend(Unsupported)` when the
    /// Machine cannot be suspended, in which case it keeps running; any other failure after the
    /// read began has ended it.
    pub fn suspend_machine(
        &mut self,
        request: &crate::SuspendRequest,
    ) -> Result<crate::Suspended, crate::SuspensionFailure> {
        let started = std::time::Instant::now();
        let (id, staging) = self
            .suspensions
            .stage()
            .map_err(|error| crate::SuspensionFailure::Store(error.to_string()))?;
        let suspended = match self
            .engine
            .suspend_machine(&crate::suspension::engine_request(request, &staging))
        {
            Ok(suspended) => suspended,
            Err(failure) => {
                let _ignored = std::fs::remove_dir_all(&staging);
                return Err(failure.into());
            }
        };
        // A read that started and failed ends the Instance with a Failed receipt: the machine is
        // gone and there is nothing to seal.
        if suspended.receipt().terminal_status() != &soma::TerminalStatus::Suspended {
            let _ignored = std::fs::remove_dir_all(&staging);
            return Err(crate::SuspensionFailure::Lost(Box::new(
                suspended.receipt().clone(),
            )));
        }
        let read = started.elapsed();
        let sealed = self
            .suspensions
            .seal(id, &staging, request, suspended.receipt());
        eprintln!(
            "soma: suspension of {} read in {} ms, sealed in {} ms",
            request.instance_id.as_str(),
            read.as_millis(),
            started.elapsed().saturating_sub(read).as_millis()
        );
        sealed
    }

    /// Resumes a Suspension as a new managed Machine, then destroys the Suspension.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; the Suspension is kept whenever no Machine resumed from it.
    pub fn resume_machine(
        &mut self,
        request: &crate::ResumeRequest,
    ) -> Result<MachineLaunch, crate::SuspensionFailure> {
        let opened = self.suspensions.open(request)?;
        let launched = self.engine.launch_machine(opened.launch(request));
        // The launch removes the members once it holds them; a refusal before that leaves them.
        let _ignored = std::fs::remove_dir_all(&opened.directory);
        let launch = launched?;
        // One Suspension resumes once: a second Instance from the same members would share a
        // private head's history and every secret the first one held.
        let _ignored = self.suspensions.destroy(&opened.id.hex());
        Ok(launch)
    }

    /// Destroys a Suspension without resuming it.
    ///
    /// # Errors
    ///
    /// Returns the typed failure when the identity is malformed or the store refuses.
    pub fn destroy_suspension(&self, suspension_id: &str) -> Result<(), crate::SuspensionFailure> {
        self.suspensions.destroy(suspension_id)
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
