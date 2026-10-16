//! Driving one command on a hosted machine, buffered or streaming.

use std::{io::BufReader, path::Path};

use soma::{BackendFailureKind, InstanceId};

use crate::backend::kvm::evidence::CommandParts;

use super::transport::{ask, open};
use super::{
    EXECUTE_CEILING, Executed, HostFailure, channel,
    wire::{Answer, Call},
};

/// Claims the host that will hold one machine, and returns what its launch established.
///
/// The host process exists before the machine, so a live machine never belongs to a process that
/// is about to exit.
pub(in crate::backend::kvm) fn execute(
    directory: &Path,
    instance: &InstanceId,
    parts: CommandParts,
) -> Result<Executed, HostFailure> {
    let CommandParts {
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
        context,
    } = parts;
    let call = if context.is_empty() {
        Call::Execute {
            instance_id: instance.clone(),
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
        }
    } else {
        Call::ExecuteInContext {
            instance_id: instance.clone(),
            program,
            arguments,
            timeout_ms,
            max_output_bytes,
            context,
        }
    };
    match ask(directory, instance, &call, EXECUTE_CEILING)? {
        Answer::Executed {
            status,
            stdout,
            stderr,
        } => Ok(Executed {
            status,
            stdout,
            stderr,
        }),
        Answer::Refused(refusal) => Err(HostFailure::Refused(refusal.into())),
        _ => Err(HostFailure::Refused(BackendFailureKind::GuestFailure)),
    }
}

/// Runs a command on the hosted machine, delivering each output chunk to `sink` as the host
/// forwards it.
///
/// The read timeout applies to each line, so a command that stays silent longer than the ceiling
/// fails the way a buffered command that outlives it does. The chunks are checked against the
/// byte counts the final line names; a host that sends more or fewer is not trusted.
pub(in crate::backend::kvm) fn execute_streaming(
    directory: &Path,
    instance: &InstanceId,
    parts: CommandParts,
    sink: soma::OutputSink<'_>,
) -> Result<Executed, HostFailure> {
    let CommandParts {
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
        context,
    } = parts;
    let call = Call::ExecuteStreaming {
        instance_id: instance.clone(),
        program,
        arguments,
        timeout_ms,
        max_output_bytes,
        context,
    };
    let unavailable = || HostFailure::Refused(BackendFailureKind::Unavailable);
    let mut stream = open(directory, instance, EXECUTE_CEILING)?;
    channel::write_line(&mut stream, &call).map_err(|()| unavailable())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|_| unavailable())?);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    loop {
        match channel::read_line::<Answer>(&mut reader).ok_or_else(unavailable)? {
            Answer::Chunk { channel, bytes } => {
                if (stdout.len() + stderr.len() + bytes.len()) as u64 > max_output_bytes {
                    return Err(HostFailure::Refused(BackendFailureKind::GuestFailure));
                }
                let channel = soma::OutputChannel::from(channel);
                sink(channel, &bytes);
                match channel {
                    soma::OutputChannel::Stdout => stdout.extend_from_slice(&bytes),
                    soma::OutputChannel::Stderr => stderr.extend_from_slice(&bytes),
                }
            }
            Answer::StreamEnded {
                status,
                stdout_bytes,
                stderr_bytes,
            } => {
                if stdout.len() as u64 != stdout_bytes || stderr.len() as u64 != stderr_bytes {
                    return Err(HostFailure::Refused(BackendFailureKind::GuestFailure));
                }
                return Ok(Executed {
                    status,
                    stdout,
                    stderr,
                });
            }
            Answer::Refused(refusal) => return Err(HostFailure::Refused(refusal.into())),
            _ => return Err(HostFailure::Refused(BackendFailureKind::GuestFailure)),
        }
    }
}
