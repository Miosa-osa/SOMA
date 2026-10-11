//! Both ends of a tunnel to a guest port.
//!
//! The caller asks the host that holds the sandbox. The host has the broker dial the guest from
//! inside its namespace, answers with one line, and from then on its end of that connection
//! carries the guest's stream: two threads copy bytes each way until either side closes.
//!
//! The answer is read a byte at a time on the caller's side, so the line is consumed exactly and
//! the first bytes of the stream are never swallowed into a read buffer that is then dropped.

use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use soma::InstanceId;

use super::{Answer, Call, HostFailure, channel, open};
use crate::TunnelFailure;
use crate::backend::kvm::KvmBackend;

/// How long a caller waits for the host to dial the guest and answer.
const OPEN_CEILING: Duration = Duration::from_secs(5);
/// The longest answer line either side accepts.
const MAX_ANSWER: usize = 4096;
const CHUNK: usize = 16 * 1024;

/// Asks the host holding `instance` for a stream to `port` on its guest.
pub(in crate::backend) fn open_tunnel(
    directory: &Path,
    instance: &InstanceId,
    port: u16,
) -> Result<UnixStream, TunnelFailure> {
    let mut stream = open(directory, instance, OPEN_CEILING).map_err(|failure| match failure {
        HostFailure::Absent => TunnelFailure::NotFound,
        HostFailure::Refused(_) => TunnelFailure::Unavailable,
    })?;
    let call = Call::Tunnel {
        instance_id: instance.clone(),
        port,
    };
    channel::write_line(&mut stream, &call).map_err(|()| TunnelFailure::Unavailable)?;
    let line = read_exact_line(&mut stream).ok_or(TunnelFailure::Unavailable)?;
    match serde_json::from_slice::<Answer>(&line) {
        Ok(Answer::Tunneled) => {
            // The stream may idle for as long as its user does; the host ends it on close.
            stream
                .set_read_timeout(None)
                .and_then(|()| stream.set_write_timeout(None))
                .map_err(|_| TunnelFailure::Unavailable)?;
            Ok(stream)
        }
        Ok(Answer::TunnelRefused(failure)) => Err(failure),
        _ => Err(TunnelFailure::Unavailable),
    }
}

/// Reads up to and including the first newline, and no further.
fn read_exact_line(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    while line.len() < MAX_ANSWER {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        if byte[0] == b'\n' {
            return Some(line);
        }
        line.push(byte[0]);
    }
    None
}

/// Serves one tunnel call on the host: dial, answer, then pump in the background.
///
/// `activity` is stamped as bytes move, so a stream that is in use keeps the host from being
/// reaped as idle.
pub(super) fn serve(
    backend: &mut KvmBackend,
    mut caller: UnixStream,
    target: (&InstanceId, u16),
    activity: &Arc<AtomicU64>,
    started: Instant,
) {
    let (instance, port) = target;
    let (answer, guest) = match backend.connect_resident(instance, port) {
        Ok(guest) => (Answer::Tunneled, Some(guest)),
        Err(failure) => (Answer::TunnelRefused(failure), None),
    };
    if channel::write_line(&mut caller, &answer).is_err() {
        return;
    }
    if let Some(guest) = guest {
        pump(caller, guest, Arc::clone(activity), started);
    }
}

/// Copies bytes between the caller and the guest until either side closes.
fn pump(caller: UnixStream, guest: OwnedFd, activity: Arc<AtomicU64>, started: Instant) {
    let guest = TcpStream::from(guest);
    let (Ok(caller_read), Ok(guest_read)) = (caller.try_clone(), guest.try_clone()) else {
        return;
    };
    let up = Arc::clone(&activity);
    thread::spawn(move || {
        copy(caller_read, guest.try_clone().ok(), &up, started);
        let _ = guest.shutdown(Shutdown::Both);
    });
    thread::spawn(move || {
        copy(guest_read, caller.try_clone().ok(), &activity, started);
        let _ = caller.shutdown(Shutdown::Both);
    });
}

/// Moves bytes from `from` to `to` until end of stream or an error.
fn copy(mut from: impl Read, to: Option<impl Write>, activity: &AtomicU64, started: Instant) {
    let Some(mut to) = to else {
        return;
    };
    let mut buffer = vec![0_u8; CHUNK];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => {
                activity.store(started.elapsed().as_secs(), Ordering::Relaxed);
                if to.write_all(&buffer[..count]).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        net::TcpListener,
    };

    use super::*;

    fn instance() -> InstanceId {
        InstanceId::new("89db112753324c3e890ef78b74381aa5").expect("instance")
    }

    /// A host that answers one tunnel call by connecting to `target`, or by refusing it.
    fn fake_host(directory: &Path, target: Option<std::net::SocketAddr>, refusal: TunnelFailure) {
        channel::prepare_directory(directory).expect("directory");
        let listener = channel::bind(&channel::socket_path(directory, &instance())).expect("bind");
        thread::spawn(move || {
            let (mut caller, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(caller.try_clone().expect("clone"))
                .read_line(&mut line)
                .expect("call");
            assert!(line.contains("tunnel"), "{line}");
            let Some(target) = target else {
                channel::write_line(&mut caller, &Answer::TunnelRefused(refusal)).expect("answer");
                return;
            };
            let guest = TcpStream::connect(target).expect("guest");
            channel::write_line(&mut caller, &Answer::Tunneled).expect("answer");
            // Bytes sent right behind the answer line must reach the caller's stream.
            caller.write_all(b"banner").expect("banner");
            pump(
                caller,
                OwnedFd::from(guest),
                Arc::new(AtomicU64::new(0)),
                Instant::now(),
            );
        });
    }

    fn echo_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0_u8; 1024];
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(count) => {
                        if stream.write_all(&buffer[..count]).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        address
    }

    #[test]
    fn a_tunnel_carries_bytes_both_ways_and_loses_none_behind_the_answer_line() {
        let directory = tempfile::tempdir().expect("directory");
        fake_host(
            directory.path(),
            Some(echo_server()),
            TunnelFailure::Unavailable,
        );
        let mut stream = open_tunnel(directory.path(), &instance(), 9223).expect("tunnel");
        let mut banner = [0_u8; 6];
        stream.read_exact(&mut banner).expect("banner");
        assert_eq!(&banner, b"banner");
        stream.write_all(b"ping").expect("write");
        let mut echoed = [0_u8; 4];
        stream.read_exact(&mut echoed).expect("echo");
        assert_eq!(&echoed, b"ping");
        // Closing the caller's end closes the guest's: the echo server reads end of stream.
        stream.shutdown(Shutdown::Both).expect("shutdown");
    }

    #[test]
    fn a_refusal_is_the_typed_failure_and_no_stream() {
        for failure in [
            TunnelFailure::PortNotAllowed,
            TunnelFailure::NoNetwork,
            TunnelFailure::Unreachable,
        ] {
            let directory = tempfile::tempdir().expect("directory");
            fake_host(directory.path(), None, failure);
            assert_eq!(
                open_tunnel(directory.path(), &instance(), 22).err(),
                Some(failure)
            );
        }
    }

    #[test]
    fn a_host_that_is_not_there_is_not_found() {
        let directory = tempfile::tempdir().expect("directory");
        assert_eq!(
            open_tunnel(directory.path(), &instance(), 9223).err(),
            Some(TunnelFailure::NotFound)
        );
    }

    #[test]
    fn an_endless_answer_line_is_refused_rather_than_buffered() {
        let (mut near, mut far) = UnixStream::pair().expect("pair");
        thread::spawn(move || {
            let _ = far.write_all(&vec![b'x'; MAX_ANSWER * 2]);
        });
        assert_eq!(read_exact_line(&mut near), None);
    }
}
