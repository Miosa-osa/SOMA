//! Suspending an Instance on the KVM backend (ADR 0034): whichever process holds the machine
//! has the guest retire its session, reads the machine into the staging directory, and releases
//! it. The engine cleans up what is left (the network lease, the host process) afterwards.

use std::path::PathBuf;

use soma::{BackendFailure, BackendFailureKind, InstanceId, SuspendObservation, SuspendRequest};

use super::KvmBackend;
use super::host::{self, HostFailure};

impl KvmBackend {
    /// Suspends one exact Instance into `request`'s staging directory.
    ///
    /// # Errors
    ///
    /// Returns a typed backend failure when the suspension started and did not finish.
    pub(in crate::backend) fn suspend(
        &mut self,
        request: SuspendRequest<'_>,
    ) -> Result<SuspendObservation, BackendFailure> {
        let operation = request.operation_id();
        let instance = request.instance_id().clone();
        let staging = request.staging().to_path_buf();
        let outcome = match self.hosted_directory() {
            None => self.suspend_resident(&instance, staging),
            Some(directory) => match host::suspend(&directory, &instance, staging) {
                Ok(suspended) => Ok(suspended),
                Err(failure @ HostFailure::Absent) => Err(self.host_kind(failure, &instance)),
                Err(HostFailure::Refused(kind)) => Err(kind),
            },
        };
        match outcome {
            Ok(true) => Ok(SuspendObservation::Suspended),
            Ok(false) => Ok(SuspendObservation::Refused),
            Err(kind) => Err(self.fail(operation, kind)),
        }
    }

    /// Suspends a machine this process drives; `false` is a refusal with nothing done.
    ///
    /// # Errors
    ///
    /// Returns the failure kind when the suspension started and did not finish.
    pub(in crate::backend::kvm) fn suspend_resident(
        &mut self,
        instance: &InstanceId,
        staging: PathBuf,
    ) -> Result<bool, BackendFailureKind> {
        let absent = self.absent_kind(instance);
        let Some(live) = self.live_for(instance) else {
            return Err(absent);
        };
        live.held.suspend(staging)
    }
}
