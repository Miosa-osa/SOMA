//! One call at a time: reading it, answering it, and the release that ends the host.

use std::{io::BufReader, os::unix::net::UnixStream};

use soma::{BackendFailureKind, InstanceId};
use soma_guest::GuestCommand;

use crate::backend::kvm::{KvmBackend, lifecycle::Force};

use super::super::{
    channel,
    wire::{Answer, Call},
};
use super::REQUEST_CEILING;

pub(super) fn receive(stream: &mut UnixStream) -> Option<Call> {
    stream.set_read_timeout(Some(REQUEST_CEILING)).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    channel::read_line::<Call>(&mut reader)
}

pub(super) fn perform(backend: &mut KvmBackend, call: Call) -> Answer {
    match call {
        Call::Execute {
            instance_id,
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
        } => {
            let Ok(command) = GuestCommand::new(program, arguments, timeout_ms, max_output_bytes)
            else {
                return Answer::Refused(BackendFailureKind::WorkloadRejected.into());
            };
            match backend.execute_resident(&instance_id, command) {
                Ok((status, stdout, stderr)) => Answer::Executed {
                    status,
                    stdout,
                    stderr,
                },
                Err(kind) => Answer::Refused(kind.into()),
            }
        }
        Call::ExecuteInContext {
            instance_id,
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
            context,
        } => {
            let Ok(command) = GuestCommand::new(program, arguments, timeout_ms, max_output_bytes)
                .and_then(|command| context.apply(command))
            else {
                return Answer::Refused(BackendFailureKind::WorkloadRejected.into());
            };
            match backend.execute_resident(&instance_id, command) {
                Ok((status, stdout, stderr)) => Answer::Executed {
                    status,
                    stdout,
                    stderr,
                },
                Err(kind) => Answer::Refused(kind.into()),
            }
        }
        Call::File {
            instance_id,
            operation,
        } => match backend.file_resident(&instance_id, &operation) {
            Ok(answer) => Answer::FileAnswered { answer },
            Err(kind) => Answer::Refused(kind.into()),
        },
        Call::Pty {
            instance_id,
            operation,
        } => match backend.pty_resident(&instance_id, &operation) {
            Ok(answer) => Answer::PtyAnswered { answer },
            Err(kind) => Answer::Refused(kind.into()),
        },
        Call::Inspect { instance_id } => match backend.inspect_resident(&instance_id) {
            Ok((state, network)) => Answer::Inspected { state, network },
            Err(kind) => Answer::Refused(kind.into()),
        },
        // Both terminal calls are decided by the loop rather than here, because each ends the
        // process and only one of them answers first.
        Call::Resize {
            instance_id,
            request,
        } => match backend.resize_resident(&instance_id, request) {
            Ok(outcome) => Answer::Resized(outcome),
            Err(failure) => Answer::ResizeRefused(failure),
        },
        Call::Cleanup { .. }
        | Call::Shutdown
        | Call::Tunnel { .. }
        | Call::ExecuteStreaming { .. } => Answer::Refused(BackendFailureKind::Unsupported.into()),
    }
}

pub(super) const fn force_for(forced: bool) -> Force {
    if forced {
        Force::Immediately
    } else {
        Force::OnlyIfTheGuestWillNotLeave
    }
}

pub(super) fn release(backend: &mut KvmBackend, instance: &InstanceId, force: Force) -> Answer {
    match backend.cleanup_resident(instance, force) {
        Ok(evidence) => Answer::Cleaned { evidence },
        Err(kind) => Answer::Refused(kind.into()),
    }
}
