//! Translation between portable terminal operations and the guest protocol.

use soma::{PtyAnswer, PtyOperation, PtyRefusal};
use soma_guest::{PtyFailure, PtyOutcome, PtyRequest as GuestPtyRequest, PtySize as GuestPtySize};

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
