use soma::{
    DestroyMachineRequest, ExecuteMachineRequest, ExecutionReceipt, FileMachineRequest,
    InspectMachineRequest, LaunchMachineRequest, MachineListing, ManagedFailure, PtyMachineRequest,
    StopMachineRequest,
};
use soma_local::{LocalFailure, LocalRuntime, LocalRuntimeConfig};

use crate::facade::{
    CommandOutcome, FileOutcome, LifecycleOutcome, OwnedVolume, SandboxFacade, SandboxSnapshot,
    SandboxStorage, TerminalOutcome, VolumeBinding, VolumeBody, VolumeFailure, VolumeOperation,
    VolumeReport,
};
use crate::{owners::Owners, volumes::VolumeClient};

/// The production facade: a durable local runtime, driven exactly as the CLI drives it.
///
/// Each instance owns one open runtime. The server opens one per connection rather than sharing
/// a single runtime across threads, because the durable file state store is already the
/// cross-process serialization point and sharing a runtime would add a second, weaker one.
pub struct LocalFacade {
    runtime: LocalRuntime,
    volumes: VolumeClient,
    owners: Owners,
}

impl LocalFacade {
    /// Opens the configured local runtime.
    ///
    /// # Errors
    ///
    /// Returns the typed local setup failure when backend selection, probing, or state-store
    /// setup cannot be completed safely.
    pub fn open(config: LocalRuntimeConfig) -> Result<Self, LocalFailure> {
        let owners = Owners::under_state_root(config.state_root());
        Ok(Self {
            owners,
            runtime: LocalRuntime::open(config)?,
            volumes: VolumeClient::from_env(),
        })
    }

    /// Whether this facade's runtime can give a sandbox the network `policy` describes.
    #[must_use]
    pub fn serves_network(&self, policy: &soma::NetworkPolicy) -> bool {
        self.runtime.serves_network(policy)
    }
}

mod workspace;

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

    fn restore_owner(
        &mut self,
        instance: &soma::InstanceId,
    ) -> Result<Option<soma::MachineName>, ManagedFailure> {
        self.runtime.restore_owner(instance)
    }

    fn restore(
        &mut self,
        request: LaunchMachineRequest,
        checkpoint: &soma::OperationId,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.restore_machine(request, checkpoint)?;
        Ok(lifecycle(outcome.receipt()))
    }

    fn fork(
        &mut self,
        request: LaunchMachineRequest,
        source: &soma::InstanceId,
        checkpoint: &soma::OperationId,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.fork_machine(request, source, checkpoint)?;
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
            machine_lost: outcome.machine_lost(),
            shut_down_by_guest: self.runtime.shut_down_by_guest(receipt.instance_id()),
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

    fn execute_streaming(
        &mut self,
        request: ExecuteMachineRequest,
        sink: soma::OutputSink<'_>,
    ) -> Result<CommandOutcome, ManagedFailure> {
        let outcome = self.runtime.execute_machine_streaming(request, sink)?;
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

    fn terminal(&mut self, request: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        let outcome = self.runtime.pty_machine(request)?;
        Ok(TerminalOutcome {
            instance_id: outcome.instance_id().clone(),
            operation: outcome.operation().name(),
            answer: outcome.answer().clone(),
        })
    }

    fn list(&mut self) -> Result<MachineListing, ManagedFailure> {
        self.runtime.list_machines_reporting_unreadable()
    }

    fn terminal_census(&mut self) -> Result<soma::TerminalCensus, ManagedFailure> {
        self.runtime.terminal_census()
    }

    fn release(
        &mut self,
        instance_id: &soma::InstanceId,
    ) -> Result<soma::MachineRelease, ManagedFailure> {
        self.runtime.release_machine(instance_id)
    }

    fn stop(&mut self, request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.stop_machine(request)?;
        Ok(lifecycle(outcome.receipt()))
    }

    fn suspend(
        &mut self,
        request: &soma_local::SuspendRequest,
    ) -> Result<soma_local::Suspended, soma_local::SuspensionFailure> {
        self.runtime.suspend_machine(request)
    }

    fn resume_suspension(
        &mut self,
        request: &soma_local::ResumeRequest,
    ) -> Result<LifecycleOutcome, soma_local::SuspensionFailure> {
        let outcome = self.runtime.resume_suspension(request)?;
        Ok(lifecycle(outcome.receipt()))
    }

    fn capacity(&mut self) -> Option<soma_local::capacity::HostCapacity> {
        Some(self.runtime.capacity())
    }

    fn discard_suspension(
        &mut self,
        suspension_id: &str,
    ) -> Result<(), soma_local::SuspensionFailure> {
        self.runtime.destroy_suspension(suspension_id)
    }

    fn destroy(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        let outcome = self.runtime.destroy_machine(request)?;
        Ok(lifecycle(outcome.receipt()))
    }

    fn pause(&mut self, instance_id: &soma::InstanceId) -> Result<(), soma_local::PauseFailure> {
        self.runtime.pause_machine(instance_id)
    }

    fn resume(&mut self, instance_id: &soma::InstanceId) -> Result<(), soma_local::ResumeFailure> {
        self.runtime.resume_machine(instance_id)
    }

    fn resize(
        &mut self,
        instance_id: &soma::InstanceId,
        request: soma_local::ResizeRequest,
    ) -> Result<soma_local::ResizeOutcome, soma_local::ResizeFailure> {
        self.runtime.resize_machine(instance_id, request)
    }

    fn set_egress(
        &mut self,
        instance_id: &soma::InstanceId,
        request: soma_local::EgressSetRequest,
    ) -> Result<soma_local::EgressSetOutcome, soma_local::EgressSetFailure> {
        self.runtime.set_egress_machine(instance_id, request)
    }

    #[cfg(unix)]
    fn tunnel(
        &mut self,
        instance_id: &soma::InstanceId,
        port: u16,
    ) -> Result<crate::tunnel::GuestStream, soma_local::TunnelFailure> {
        self.runtime.tunnel_machine(instance_id, port)
    }
}

impl SandboxStorage for LocalFacade {
    fn owners(&self) -> Option<&Owners> {
        Some(&self.owners)
    }

    fn checkpoint(
        &mut self,
        instance: &soma::InstanceId,
        operation: &soma::OperationId,
    ) -> Result<soma_local::CheckpointReceipt, soma_local::CheckpointFailure> {
        self.runtime.checkpoint_machine(instance, operation)
    }

    fn discard_checkpoint(
        &mut self,
        instance: &soma::InstanceId,
        checkpoint: &soma::OperationId,
        state_digest: &soma::GenerationId,
    ) -> Result<(), soma_local::CheckpointFailure> {
        self.runtime
            .discard_checkpoint(instance, checkpoint, state_digest)
    }

    /// The volume store lives in the host daemon, so this forwards to the control socket and
    /// reports the missing capability when no socket is configured.
    fn volume_binding(&mut self, instance: &str) -> Result<VolumeBinding, VolumeFailure> {
        self.volumes.binding(instance)
    }

    fn volume_owned(&mut self, instance: &str) -> Result<Option<OwnedVolume>, VolumeFailure> {
        self.volumes.owned(instance)
    }

    fn volume(
        &mut self,
        operation: VolumeOperation,
        body: &VolumeBody,
    ) -> Result<VolumeReport, VolumeFailure> {
        if matches!(operation, VolumeOperation::Attach | VolumeOperation::Detach)
            && self.volumes.configured()
            && let Some(report) = self.live_workspace(operation, body)?
        {
            return Ok(report);
        }
        self.volumes.call(operation, body)
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
