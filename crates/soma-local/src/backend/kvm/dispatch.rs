//! The four Backend lifecycle operations, and which process performs each one.
//!
//! Only one decision is made here: whether this process holds the machine itself or a host
//! process holds it. Everything else is the same either way, and the evidence a caller reads is
//! always assembled on this side, from the request it made and the facts the machine reported,
//! timed by this process's own clock.

use std::{any::Any, path::PathBuf};

use soma::{
    BackendFailure, BackendFailureKind, CleanupObservation, CleanupReason, CleanupRequest,
    CleanupTimes, CommandObservation, CommandTimes, DigestBinding, ExecutionRequest,
    InspectionObservation, InspectionRequest, InstanceId, IsolationClass, LaunchObservation,
    LaunchRequest, LaunchTimes, SandboxLiveness,
};
use soma_guest::GuestCommand;

use super::{
    KvmBackend,
    evidence::{command_parts, effective_shape, observation},
    host::{self, HostFailure, Role},
    lifecycle::Force,
    prepared::PreparedGeneration,
};

impl KvmBackend {
    pub(in crate::backend) fn launch(
        &mut self,
        request: &LaunchRequest<'_, Box<dyn Any + Send>>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let operation = request.operation_id();
        let admitted = self.clocks.elapsed_ns(operation);
        let prepared = request
            .prepared()
            .downcast_ref::<PreparedGeneration>()
            .ok_or_else(|| self.fail(operation, BackendFailureKind::WorkloadRejected))?;
        let launched = match self.hosted_directory() {
            None => {
                self.launch_resident(operation, request.instance_id(), prepared, request.shape())?
            }
            Some(directory) => host::launch(
                &directory,
                operation,
                request.instance_id(),
                prepared,
                request.shape(),
            )
            .map_err(|kind| self.fail(operation, kind))?,
        };
        let ready = self.clocks.elapsed_ns(operation);
        Ok(LaunchObservation::new(
            operation.clone(),
            request.instance_id().clone(),
            request.workload().clone(),
            Self::kind(),
            IsolationClass::HardwareVirtualMachine,
            // Only a machine this Launch actually claimed from the pool is reported as
            // prepared. A depleted pool restored its own machine and says so.
            launched.preparation,
            DigestBinding::LaunchEnforced,
            effective_shape(launched.memory_mib, launched.storage_mib),
            launched.network,
            // The launch stamp is taken on the clock of whichever process built the machine, so
            // a hosted launch reports it after this operation was admitted here and never after
            // the moment this process saw the machine become ready.
            LaunchTimes::new(
                admitted,
                admitted.saturating_add(launched.at_ns).min(ready),
                ready,
            ),
        ))
    }

    pub(in crate::backend) fn execute(
        &mut self,
        request: ExecutionRequest<'_>,
    ) -> Result<CommandObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let Some(parts) = command_parts(&request) else {
            return Err(self.fail(operation, BackendFailureKind::WorkloadRejected));
        };
        let instance = request.instance_id().clone();
        let outcome = match self.hosted_directory() {
            None => GuestCommand::new(
                parts.program,
                parts.arguments,
                parts.timeout_ms,
                parts.max_output_bytes,
            )
            .map_err(|_| BackendFailureKind::WorkloadRejected)
            .and_then(|command| self.execute_resident(&instance, command)),
            Some(directory) => host::execute(
                &directory,
                &instance,
                parts.program,
                parts.arguments,
                parts.timeout_ms,
                parts.max_output_bytes,
            )
            .map(|executed| (executed.status, executed.stdout, executed.stderr))
            .map_err(|failure| self.host_kind(failure, &instance)),
        };
        let (status, stdout, stderr) = outcome.map_err(|kind| self.fail(operation, kind))?;
        let finished = self.clocks.elapsed_ns(operation);
        Ok(observation(
            &request,
            status,
            stdout,
            stderr,
            CommandTimes::new(started, finished),
        ))
    }

    pub(in crate::backend) fn inspect(
        &mut self,
        request: InspectionRequest<'_>,
    ) -> Result<InspectionObservation, BackendFailure> {
        let operation = request.operation_id();
        let observed = self.clocks.elapsed_ns(operation);
        let instance = request.instance_id().clone();
        let outcome = match self.hosted_directory() {
            None => self.inspect_resident(&instance),
            Some(directory) => host::inspect(&directory, &instance)
                .map_err(|failure| self.host_kind(failure, &instance)),
        };
        let (state, network) = outcome.map_err(|kind| self.fail(operation, kind))?;
        Ok(InspectionObservation::observed(
            request,
            Self::kind(),
            state,
            network,
            observed,
        ))
    }

    pub(in crate::backend) fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> Result<CleanupObservation, BackendFailure> {
        let operation = request.operation_id();
        let started = self.clocks.elapsed_ns(operation);
        let instance = request.instance_id().clone();
        // A forced destroy ends the machine without asking the guest. Every other reason asks
        // first, because a release the caller described as graceful must have been one.
        let force = match request.reason() {
            CleanupReason::ForcedDestroy => Force::Immediately,
            CleanupReason::RunCompleted
            | CleanupReason::GracefulStop
            | CleanupReason::Rollback
            | CleanupReason::UncertainCommandTermination => Force::OnlyIfTheGuestWillNotLeave,
        };
        let outcome = match self.hosted_directory() {
            None => self.cleanup_resident(&instance, force),
            Some(directory) => {
                match host::cleanup(&directory, &instance, matches!(force, Force::Immediately)) {
                    Ok(evidence) => Ok(evidence),
                    // No host serves this Instance, so there is no machine here to release and the
                    // one terminal act left is ending the Host Runtime's ownership of the identity.
                    Err(HostFailure::Absent) => self.release_unowned(&instance),
                    Err(HostFailure::Refused(kind)) => Err(kind),
                }
            }
        };
        let evidence = outcome.map_err(|kind| self.fail(operation, kind))?;
        let finished = self.clocks.elapsed_ns(operation);
        Ok(CleanupObservation::new(
            operation.clone(),
            instance,
            evidence,
            CleanupTimes::new(started, finished),
        ))
    }

    /// Reports whether anything is still serving one exact Instance.
    ///
    /// On the hosted path this is a connect to the Instance's own socket, which is the only
    /// process that can be holding its machine. On the resident path the answer is knowable only
    /// for a machine this process itself is driving; a record written by some other process names
    /// a machine that died with it, but nothing here observed that, so it is reported as unknown
    /// rather than asserted.
    pub(in crate::backend) fn liveness(&mut self, instance: &InstanceId) -> SandboxLiveness {
        match self.hosted_directory() {
            Some(directory) => host::liveness(&directory, instance),
            None if self.live_for(instance).is_some() => SandboxLiveness::Live,
            None => SandboxLiveness::Unknown,
        }
    }

    /// Where hosted machines are addressed, or nothing when this process holds its own.
    pub(super) fn hosted_directory(&self) -> Option<PathBuf> {
        match &self.role {
            Role::Resident | Role::MachineHost => None,
            Role::Hosted(directory) => Some(directory.clone()),
        }
    }

    /// The failure kind for an operation that never reached a host.
    pub(super) fn host_kind(
        &self,
        failure: HostFailure,
        instance: &InstanceId,
    ) -> BackendFailureKind {
        match failure {
            HostFailure::Absent => self.absent_kind(instance),
            HostFailure::Refused(kind) => kind,
        }
    }
}
