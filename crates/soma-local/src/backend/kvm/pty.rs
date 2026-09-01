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
    PtyRefusal, PtyRequest,
};
use soma_guest::{PtyFailure, PtyOutcome, PtyRequest as GuestPtyRequest, PtySize as GuestPtySize};

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

/// The one guest request this operation is.
///
/// A size the guest protocol will not carry produces nothing here rather than a clamped request.
/// Clamping would give two different calls the same meaning, and the caller asked for one of
/// them. Every public surface refuses such a call before it reaches this point; this returns
/// nothing so that a mapping defect cannot become an invented request either.
pub(super) fn guest_request(operation: &PtyOperation) -> Option<GuestPtyRequest> {
    Some(match operation {
        PtyOperation::Open { columns, rows } => {
            GuestPtyRequest::Open(GuestPtySize::new(*columns, *rows).ok()?)
        }
        PtyOperation::Resize { columns, rows } => {
            GuestPtyRequest::Resize(GuestPtySize::new(*columns, *rows).ok()?)
        }
        PtyOperation::Write { bytes } => GuestPtyRequest::Write {
            bytes: bytes.as_slice().into(),
        },
        PtyOperation::Read { wait_millis } => GuestPtyRequest::Read {
            wait_millis: *wait_millis,
        },
        PtyOperation::Close => GuestPtyRequest::Close,
    })
}

/// What one guest answer becomes on the portable side.
///
/// The guest's closed failure set is carried across one to one. Nothing widens it: a cause the
/// host invented would tell a caller the guest said something it did not.
pub(super) fn answer_from(outcome: PtyOutcome) -> PtyAnswer {
    match outcome {
        PtyOutcome::Opened(size) => PtyAnswer::Opened {
            columns: size.columns(),
            rows: size.rows(),
        },
        PtyOutcome::Wrote { bytes } => PtyAnswer::Wrote { bytes },
        PtyOutcome::Output { bytes, end } => PtyAnswer::Output {
            bytes: bytes.into_vec(),
            end,
        },
        PtyOutcome::Resized(size) => PtyAnswer::Resized {
            columns: size.columns(),
            rows: size.rows(),
        },
        PtyOutcome::Closed => PtyAnswer::Closed,
        PtyOutcome::Failed(failure) => PtyAnswer::Refused(refusal_from(failure)),
    }
}

const fn refusal_from(failure: PtyFailure) -> PtyRefusal {
    match failure {
        PtyFailure::NoSession => PtyRefusal::NoSession,
        PtyFailure::AlreadyOpen => PtyRefusal::AlreadyOpen,
        PtyFailure::Denied => PtyRefusal::Denied,
        PtyFailure::Failed => PtyRefusal::Failed,
    }
}
