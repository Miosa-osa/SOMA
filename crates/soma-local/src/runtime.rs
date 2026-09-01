use soma::{
    Backend as _, BackendKind, DestroyMachineRequest, Engine, ExecuteMachineRequest,
    FileMachineRequest, LaunchMachineRequest, MachineDestroy, MachineExecution, MachineFile,
    MachineInspection, MachineLaunch, MachineStop, ManagedFailure, RunFailure, RunOutcome,
    RunRequest, StopMachineRequest,
};

use crate::{
    BackendSelection, FileStateStore, LocalFailure, LocalFailureKind, LocalRuntimeConfig,
    backend::LocalBackend,
};

pub struct LocalRuntime {
    engine: Engine<LocalBackend, FileStateStore>,
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
            LocalBackend::open(config.backend, config.runtime, host_directory)?;
        let backend_kind = backend.kind();
        let state = FileStateStore::open(config.state_root)
            .map_err(|failure| LocalFailure::new(LocalFailureKind::StateStore(failure.kind())))?;
        Ok(Self {
            engine: Engine::with_state_store(backend, state),
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
