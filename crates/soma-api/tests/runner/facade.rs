//! A facade standing in for KVM, counting what the runner asked of it.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use soma::{
    DestroyMachineRequest, ExecuteMachineRequest, FileMachineRequest, InspectMachineRequest,
    LaunchMachineRequest, ManagedFailure, PtyMachineRequest, SandboxEntry, StopMachineRequest,
    TerminalStatus,
};
use soma_api::{
    CommandOutcome, FileOutcome, LifecycleOutcome, SandboxFacade, SandboxSnapshot, TerminalOutcome,
    runner::FacadeOpener,
};

use crate::support::receipt;

#[derive(Default)]
pub(crate) struct Engine {
    pub(crate) launches: AtomicUsize,
    pub(crate) executes: AtomicUsize,
    pub(crate) destroys: AtomicUsize,
    pub(crate) inspects: AtomicUsize,
}

pub(crate) struct FakeFacade(Arc<Engine>);

impl SandboxFacade for FakeFacade {
    fn hosts_addressable_sandboxes(&self) -> bool {
        true
    }

    fn launch(&mut self, _: LaunchMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.0.launches.fetch_add(1, Ordering::SeqCst);
        let receipt = receipt();
        Ok(LifecycleOutcome {
            instance_id: receipt.instance_id().clone(),
            receipt,
        })
    }

    fn inspect(&mut self, _: InspectMachineRequest) -> Result<SandboxSnapshot, ManagedFailure> {
        self.0.inspects.fetch_add(1, Ordering::SeqCst);
        let receipt = receipt();
        Ok(SandboxSnapshot {
            instance_id: receipt.instance_id().clone(),
            state: soma::MachineState::Ready,
            backend: receipt.backend(),
            receipt,
        })
    }

    fn execute(&mut self, _: ExecuteMachineRequest) -> Result<CommandOutcome, ManagedFailure> {
        self.0.executes.fetch_add(1, Ordering::SeqCst);
        let receipt = receipt();
        Ok(CommandOutcome {
            instance_id: receipt.instance_id().clone(),
            status: TerminalStatus::Exited { code: 0 },
            stdout: b"v22.23.2\n".to_vec(),
            stderr: Vec::new(),
            receipt,
        })
    }

    fn file(&mut self, _: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        unreachable!("the runner never touches files")
    }

    fn terminal(&mut self, _: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        unreachable!("the runner never opens terminals")
    }

    fn list(&mut self) -> Result<Vec<SandboxEntry>, ManagedFailure> {
        Ok(Vec::new())
    }

    fn stop(&mut self, _: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        unreachable!("the runner never stops")
    }

    fn destroy(&mut self, _: DestroyMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.0.destroys.fetch_add(1, Ordering::SeqCst);
        let receipt = receipt();
        Ok(LifecycleOutcome {
            instance_id: receipt.instance_id().clone(),
            receipt,
        })
    }
}

pub(crate) fn opener(engine: &Arc<Engine>) -> FacadeOpener {
    let engine = Arc::clone(engine);
    Arc::new(move || Ok(Box::new(FakeFacade(Arc::clone(&engine))) as Box<dyn SandboxFacade>))
}
