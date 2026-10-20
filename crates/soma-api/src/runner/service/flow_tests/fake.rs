//! The flow double's facade implementation.
//!
//! It lives apart from the double's fields so the tests stay inside the source budget; a child
//! module may read them, so the split costs nothing but a `use`.

#[allow(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

fn lifecycle() -> LifecycleOutcome {
    let receipt: ExecutionReceipt = serde_json::from_str(RECEIPT).expect("receipt");
    LifecycleOutcome {
        instance_id: receipt.instance_id().clone(),
        receipt,
    }
}

impl SandboxFacade for Fake {
    fn hosts_addressable_sandboxes(&self) -> bool {
        true
    }

    fn launch(&mut self, _: LaunchMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.0.launches.fetch_add(1, Ordering::SeqCst);
        // Held before the machine is built, so a test can watch the answer a create gives while
        // its launch is still in flight.
        if let Some(gate) = self.0.launch_gate.lock().expect("gate").take() {
            let _ = gate.recv();
        }
        if let Some(kind) = self.0.launch_refusal.lock().expect("refusal").take() {
            return Err(ManagedFailure::Backend(kind));
        }
        Ok(lifecycle())
    }

    fn set_egress(
        &mut self,
        _: &soma::InstanceId,
        request: soma_local::EgressSetRequest,
    ) -> Result<soma_local::EgressSetOutcome, soma_local::EgressSetFailure> {
        self.0.egress_calls.lock().expect("calls").push(request);
        match self.0.egress_script.lock().expect("script").pop_front() {
            Some(scripted) => scripted,
            None => Ok(soma_local::EgressSetOutcome {
                open: request.open,
                dns: request.dns,
                changed: true,
            }),
        }
    }

    fn inspect(&mut self, _: InspectMachineRequest) -> Result<SandboxSnapshot, ManagedFailure> {
        unreachable!("not a runner route")
    }

    fn execute(&mut self, _: ExecuteMachineRequest) -> Result<CommandOutcome, ManagedFailure> {
        self.0.execs.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(*self.0.exec_delay.lock().expect("delay"));
        if let Some(kind) = self.0.exec_refusal.lock().expect("refusal").take() {
            return Err(ManagedFailure::Backend(kind));
        }
        let outcome = lifecycle();
        let status = self
            .0
            .exec_status
            .lock()
            .expect("status")
            .take()
            .unwrap_or(TerminalStatus::Exited { code: 3 });
        Ok(CommandOutcome {
            instance_id: outcome.instance_id,
            status,
            stdout: Vec::new(),
            stderr: b"no".to_vec(),
            receipt: outcome.receipt,
        })
    }

    fn execute_streaming(
        &mut self,
        request: ExecuteMachineRequest,
        sink: soma::OutputSink<'_>,
    ) -> Result<CommandOutcome, ManagedFailure> {
        let script = self.0.stream_script.lock().expect("script").take();
        let Some(script) = script else {
            // Unscripted commands behave as they always did: run, then replay what was retained.
            let outcome = self.execute(request)?;
            soma::ObservedOutput::new(
                outcome.stdout.clone(),
                outcome.stdout.len() as u64,
                outcome.stderr.clone(),
                outcome.stderr.len() as u64,
            )
            .replay(sink);
            return Ok(outcome);
        };
        self.0.execs.fetch_add(1, Ordering::SeqCst);
        for (index, (channel, bytes)) in script.chunks.iter().enumerate() {
            if index == 1 {
                let _ = script.gate.recv();
            }
            sink(*channel, bytes);
        }
        let outcome = lifecycle();
        Ok(CommandOutcome {
            instance_id: outcome.instance_id,
            status: TerminalStatus::Exited { code: 0 },
            stdout: Vec::new(),
            stderr: Vec::new(),
            receipt: outcome.receipt,
        })
    }

    fn file(&mut self, _: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        unreachable!("not a runner route")
    }

    fn terminal(&mut self, _: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        std::thread::sleep(*self.0.terminal_delay.lock().expect("delay"));
        Ok(TerminalOutcome {
            instance_id: lifecycle().instance_id,
            operation: "read",
            answer: soma::PtyAnswer::Output {
                bytes: b"$ ".to_vec(),
                end: false,
            },
        })
    }

    fn list(&mut self) -> Result<soma::MachineListing, ManagedFailure> {
        Ok(soma::MachineListing::new(
            self.0.listed.lock().expect("listed").clone(),
        ))
    }

    fn stop(&mut self, _: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        unreachable!("not a runner route")
    }

    fn destroy(&mut self, _: DestroyMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.0.destroys.fetch_add(1, Ordering::SeqCst);
        Ok(lifecycle())
    }
}
