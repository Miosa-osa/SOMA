//! Resize, workspace, pause, egress, stop and release calls on a running machine.

use super::LocalRuntime;
use soma::{
    DestroyMachineRequest, MachineDestroy, MachineStop, ManagedFailure, StopMachineRequest,
};

impl LocalRuntime {
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

    /// Runs a workspace attach or detach transaction on the running Instance's owner.
    ///
    /// # Errors
    ///
    /// Refuses unavailable hosting, an unsupported guest, an active mount holder or an
    /// uncertain handoff.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn change_workspace(
        &self,
        instance: &soma::InstanceId,
        request: &soma_hostd::Request,
    ) -> Result<soma_hostd::Reply, soma::BackendFailureKind> {
        let directory = self
            .host_directory
            .as_deref()
            .ok_or(soma::BackendFailureKind::Unsupported)?;
        crate::backend::change_workspace(directory, instance, request)
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
