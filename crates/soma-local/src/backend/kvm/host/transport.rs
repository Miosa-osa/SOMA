//! Bounded request and response transport for one hosted machine.

use std::{
    io::{BufReader, Read as _},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

use soma::{BackendFailureKind, InstanceId};

use super::{Answer, Call, HostFailure, channel};

pub(super) fn ask(
    directory: &Path,
    instance: &InstanceId,
    call: &Call,
    within: Duration,
) -> Result<Answer, HostFailure> {
    let mut stream = open(directory, instance, within)?;
    exchange(&mut stream, call)
}

pub(super) fn open(
    directory: &Path,
    instance: &InstanceId,
    within: Duration,
) -> Result<UnixStream, HostFailure> {
    let stream = channel::connect(directory, instance).map_err(|()| HostFailure::Absent)?;
    stream
        .set_read_timeout(Some(within))
        .and_then(|()| stream.set_write_timeout(Some(within)))
        .map_err(|_| HostFailure::Refused(BackendFailureKind::Unavailable))?;
    Ok(stream)
}

pub(super) fn exchange(stream: &mut UnixStream, call: &Call) -> Result<Answer, HostFailure> {
    channel::write_line(stream, call)
        .map_err(|()| HostFailure::Refused(BackendFailureKind::Unavailable))?;
    let cloned = stream
        .try_clone()
        .map_err(|_| HostFailure::Refused(BackendFailureKind::Unavailable))?;
    let mut reader = BufReader::new(cloned);
    channel::read_line::<Answer>(&mut reader)
        .ok_or(HostFailure::Refused(BackendFailureKind::Unavailable))
}

/// Wait for the host to close the connection on its way out.
pub(super) fn await_close(stream: &mut UnixStream) -> Result<(), HostFailure> {
    let mut remainder = [0u8; 1];
    match stream.read(&mut remainder) {
        Ok(0) => Ok(()),
        _ => Err(HostFailure::Refused(BackendFailureKind::CleanupFailure)),
    }
}
