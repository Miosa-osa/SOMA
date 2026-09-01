//! One portable filesystem operation, carried to the guest that performs it.
//!
//! The translation between the portable operation and the guest protocol lives here and nowhere
//! else. It is a mapping, not a layer: each of the six operations becomes one guest request, or,
//! for the two that move a whole file, the bounded chunk loop the guest session already exposes.
//!
//! A path is not resolved, normalised, or rewritten on the way through. The guest validates every
//! path it is given, refuses what its own policy refuses, and is the only side that knows what
//! the sandbox's filesystem actually contains, so a host that pre-approved a path would be
//! deciding something it cannot see.

use soma::{
    BackendFailure, BackendFailureKind, FileAnswer, FileObservation, FileOperation, FileRequest,
    InstanceId,
};

use super::{KvmBackend, host};

impl KvmBackend {
    pub(in crate::backend) fn file(
        &mut self,
        request: FileRequest<'_>,
    ) -> Result<FileObservation, BackendFailure> {
        let operation = request.operation_id();
        let instance = request.instance_id().clone();
        let answer = match self.hosted_directory() {
            None => self.file_resident(&instance, request.operation()),
            Some(directory) => host::file(&directory, &instance, request.operation())
                .map_err(|failure| self.host_kind(failure, &instance)),
        };
        let answer = answer.map_err(|kind| self.fail(operation, kind))?;
        Ok(FileObservation::new(operation.clone(), instance, answer))
    }
}

impl KvmBackend {
    /// Performs the operation against the sandbox this process is driving.
    pub(super) fn file_resident(
        &mut self,
        instance: &InstanceId,
        operation: &FileOperation,
    ) -> Result<FileAnswer, BackendFailureKind> {
        let Some(live) = self.live_for(instance) else {
            return Err(self.absent_kind(instance));
        };
        live.held.file(operation.clone())
    }
}
