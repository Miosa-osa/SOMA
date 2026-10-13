//! The facade the route tests drive, as one implementation of the interface.
//!
//! It lives apart from the double's fields so the double stays inside the source budget; a child
//! module may read them, so the split costs nothing but a `use`.

use super::*;

impl SandboxFacade for FakeFacade {
    fn hosts_addressable_sandboxes(&self) -> bool {
        self.hosts_addressable_sandboxes
    }

    fn resize(
        &mut self,
        _instance_id: &soma::InstanceId,
        request: soma_api::ResizeRequest,
    ) -> Result<soma_api::ResizeOutcome, soma_api::ResizeFailure> {
        self.calls.push(Call::Resize(request));
        match (&self.mode, &self.resize_outcome) {
            (Mode::NotFound, _) => Err(soma_api::ResizeFailure::NotFound),
            (_, Some(outcome)) => Ok(outcome.clone()),
            (_, None) => Err(soma_api::ResizeFailure::Unavailable),
        }
    }

    fn tunnel(
        &mut self,
        _instance_id: &soma::InstanceId,
        port: u16,
    ) -> Result<std::os::unix::net::UnixStream, TunnelFailure> {
        self.calls.push(Call::Tunnel(port));
        self.tunnel_failure
            .map_or_else(|| Ok(echo_guest(port)), Err)
    }

    fn launch(
        &mut self,
        request: LaunchMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        self.launched_volume = request.volume().cloned();
        self.lifecycle(Call::Launch)
    }

    fn volume_binding(
        &mut self,
        instance: &str,
    ) -> Result<soma_api::volumes::VolumeBinding, soma_api::volumes::VolumeFailure> {
        self.volume_calls.push(instance.to_owned());
        self.binding
            .clone()
            .ok_or(soma_api::volumes::VolumeFailure::Refused(14))
    }

    fn inspect(
        &mut self,
        _request: InspectMachineRequest,
    ) -> Result<SandboxSnapshot, ManagedFailure> {
        self.record(Call::Inspect)?;
        let receipt = receipt();
        Ok(SandboxSnapshot {
            instance_id: receipt.instance_id().clone(),
            state: MachineState::Ready,
            backend: receipt.backend(),
            receipt,
        })
    }

    fn execute(
        &mut self,
        _request: ExecuteMachineRequest,
    ) -> Result<CommandOutcome, ManagedFailure> {
        self.record(Call::Execute)?;
        let receipt = receipt();
        Ok(CommandOutcome {
            instance_id: receipt.instance_id().clone(),
            status: *receipt.terminal_status(),
            stdout: b"v22.23.2\n".to_vec(),
            stderr: Vec::new(),
            receipt,
        })
    }

    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        self.record(Call::File)?;
        let operation = request.operation().clone();
        let answer = answer_for(&mut self.files, &operation);
        Ok(FileOutcome {
            instance_id: soma::InstanceId::new(FIXTURE_INSTANCE_ID)
                .expect("the fixture instance id is canonical"),
            operation: operation.name(),
            answer,
        })
    }

    fn terminal(&mut self, request: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        self.record(Call::Terminal)?;
        let operation = request.operation().clone();
        let answer = terminal::answer_for(&mut self.session, &operation);
        Ok(TerminalOutcome {
            instance_id: soma::InstanceId::new(FIXTURE_INSTANCE_ID)
                .expect("the fixture instance id is canonical"),
            operation: operation.name(),
            answer,
        })
    }

    fn list(&mut self) -> Result<soma::MachineListing, ManagedFailure> {
        self.record(Call::List)?;
        Ok(soma::MachineListing::with_unreadable(
            self.sandboxes.clone(),
            self.unreadable.clone(),
        ))
    }

    fn stop(&mut self, _request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.lifecycle(Call::Stop)
    }

    fn destroy(
        &mut self,
        _request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        self.lifecycle(Call::Destroy)
    }
}
