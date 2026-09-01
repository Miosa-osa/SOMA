use soma::{
    DestroyMachineRequest, ExecuteMachineRequest, ExecutionReceipt, FileMachineRequest,
    InspectMachineRequest, LaunchMachineRequest, ManagedFailure, StopMachineRequest,
};
use soma_local::{LocalFailure, LocalRuntime, LocalRuntimeConfig};

use crate::facade::{
    CommandOutcome, FileOutcome, LifecycleOutcome, SandboxFacade, SandboxSnapshot,
};

/// The production facade: a durable local runtime, driven exactly as the CLI drives it.
///
/// Each instance owns one open runtime. The server opens one per connection rather than sharing
/// a single runtime across threads, because the durable file state store is already the
/// cross-process serialization point and sharing a runtime would add a second, weaker one.
pub struct LocalFacade {
    runtime: LocalRuntime,
}

impl LocalFacade {
    /// Opens the configured local runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed local setup failure when backend selection, probing, or state-store
    /// setup cannot be completed safely.
    pub fn open(config: LocalRuntimeConfig) -> Result<Self, LocalFailure> {
        Ok(Self {
            runtime: LocalRuntime::open(config)?,
        })
    }
}

impl SandboxFacade for LocalFacade {
    fn hosts_addressable_sandboxes(&self) -> bool {
        self.runtime.machine_hosting() == soma_local::MachineHosting::OutlivesProcess
    }

    fn launch(
        &mut self,
        request: LaunchMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.launch_machine(request)?;
        Ok(lifecycle(outcome.receipt()))
    }

    fn inspect(
        &mut self,
        request: InspectMachineRequest,
    ) -> Result<SandboxSnapshot, ManagedFailure> {
        let outcome = self.runtime.inspect_machine(request)?;
        let receipt = outcome.receipt();
        Ok(SandboxSnapshot {
            instance_id: receipt.instance_id().clone(),
            state: outcome.state(),
            backend: receipt.backend(),
            receipt: receipt.clone(),
        })
    }

    fn execute(
        &mut self,
        request: ExecuteMachineRequest,
    ) -> Result<CommandOutcome, ManagedFailure> {
        let outcome = self.runtime.execute_machine(request)?;
        let receipt = outcome.receipt();
        Ok(CommandOutcome {
            instance_id: receipt.instance_id().clone(),
            status: *receipt.terminal_status(),
            stdout: outcome.output().stdout().to_vec(),
            stderr: outcome.output().stderr().to_vec(),
            receipt: receipt.clone(),
        })
    }

    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        let outcome = self.runtime.file_machine(request)?;
        Ok(FileOutcome {
            instance_id: outcome.instance_id().clone(),
            operation: outcome.operation().name(),
            answer: outcome.answer().clone(),
        })
    }

    fn stop(&mut self, request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.stop_machine(request)?;
        Ok(lifecycle(outcome.receipt()))
    }

    fn destroy(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.destroy_machine(request)?;
        Ok(lifecycle(outcome.receipt()))
    }
}

/// Reads the instance identity back out of the receipt rather than off the request.
///
/// The receipt is the engine's own account of what it acted on, so taking the identity from it
/// means the response can never name a sandbox the engine did not actually touch.
fn lifecycle(receipt: &ExecutionReceipt) -> LifecycleOutcome {
    LifecycleOutcome {
        instance_id: receipt.instance_id().clone(),
        receipt: receipt.clone(),
    }
}
