//! A facade standing in for KVM, counting what the runner asked of it.

use std::sync::{
    Arc, Mutex,
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
    pub(crate) tunnels: AtomicUsize,
    /// Why the next tunnels are refused; `None` connects them to an echoing guest.
    pub(crate) tunnel_failure: Mutex<Option<soma_api::TunnelFailure>>,
}

pub(crate) struct FakeFacade(Arc<Engine>);

impl SandboxFacade for FakeFacade {
    fn hosts_addressable_sandboxes(&self) -> bool {
        true
    }

    fn tunnel(
        &mut self,
        _instance_id: &soma::InstanceId,
        port: u16,
    ) -> Result<std::os::unix::net::UnixStream, soma_api::TunnelFailure> {
        self.0.tunnels.fetch_add(1, Ordering::SeqCst);
        match *self.0.tunnel_failure.lock().expect("failure") {
            Some(failure) => Err(failure),
            None => Ok(echo_guest(port)),
        }
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

/// The guest's service as the broker's connected socket reaches it: speaks first, then echoes
/// until told to `quit`.
fn echo_guest(port: u16) -> std::os::unix::net::UnixStream {
    use std::io::{Read, Write};
    let (service, guest) = std::os::unix::net::UnixStream::pair().expect("a socket pair");
    std::thread::spawn(move || {
        let mut guest = guest;
        if guest
            .write_all(format!("guest port {port} ready\n").as_bytes())
            .is_err()
        {
            return;
        }
        let mut buffer = [0_u8; 4096];
        loop {
            match guest.read(&mut buffer) {
                Ok(0) | Err(_) => return,
                Ok(read) => {
                    if buffer[..read].starts_with(b"quit")
                        || guest.write_all(&buffer[..read]).is_err()
                    {
                        return;
                    }
                }
            }
        }
    });
    service
}
