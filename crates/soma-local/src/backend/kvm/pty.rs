//! One portable terminal operation, carried to the guest that performs it.
//!
//! The translation between the portable operation and the guest protocol lives here and nowhere
//! else. It is a mapping, not a layer: each of the five operations becomes exactly one guest
//! request, and neither the bytes typed at the terminal nor the bytes it produced are examined,
//! rewritten, or buffered on the way through.
//!
//! Nothing here holds a session. The session belongs to the guest, and the one thing on this side
//! that has to be true for a caller in a second process to reach it is that the machine outlives
//! the process that launched it, which is what the machine host is for.

use soma::{
    BackendFailure, BackendFailureKind, InstanceId, PtyAnswer, PtyObservation, PtyOperation,
    PtyRequest,
};

use super::{KvmBackend, host, start::failure_kind};

impl KvmBackend {
    pub(in crate::backend) fn pty(
        &mut self,
        request: PtyRequest<'_>,
    ) -> Result<PtyObservation, BackendFailure> {
        let operation = request.operation_id();
        let instance = request.instance_id().clone();
        let answer = match self.hosted_directory() {
            None => self.pty_resident(&instance, request.operation()),
            Some(directory) => host::pty(&directory, &instance, request.operation())
                .map_err(|failure| self.host_kind(failure, &instance)),
        };
        let answer = answer.map_err(|kind| self.fail(operation, kind))?;
        Ok(PtyObservation::new(operation.clone(), instance, answer))
    }

    /// Performs the operation against the sandbox this process is driving.
    pub(super) fn pty_resident(
        &mut self,
        instance: &InstanceId,
        operation: &PtyOperation,
    ) -> Result<PtyAnswer, BackendFailureKind> {
        let Some(live) = self.live_for(instance) else {
            return Err(self.absent_kind(instance));
        };
        live.session.pty(operation.clone()).map_err(failure_kind)
    }
}
