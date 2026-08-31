//! A client for the Host daemon socket, and the daemon thread it talks to.
//!
//! The client speaks the exact `SOCK_SEQPACKET` frame protocol a CLI, MCP, or provider
//! adapter would speak, so a test observes the real accept, decode, and dispatch path rather
//! than an in-process shortcut, and closing a client really closes a connection.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition.
#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

use std::{
    ffi::CString,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStrExt,
    },
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use soma_hostd::{MAX_FRAME, Reply, Request, daemon};

use super::TestRuntime;

const PATIENCE: Duration = Duration::from_secs(5);

/// Serves `runtime` on `socket` and returns once the socket accepts connections.
pub fn daemon_on(runtime: &Arc<TestRuntime>, socket: &Path) {
    let served = Arc::clone(runtime);
    let path = socket.to_path_buf();
    thread::spawn(move || {
        let _ = daemon::serve(&served, &path);
    });
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && !socket.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(socket.exists(), "the daemon never bound its socket");
}

/// One connected client.
pub struct Client {
    connection: OwnedFd,
}

impl Client {
    /// Connects to the daemon socket, waiting briefly for the listener to be ready.
    #[must_use]
    pub fn connect(path: &Path) -> Self {
        let target = CString::new(path.as_os_str().as_bytes()).expect("socket path");
        // SAFETY: `socket` has no memory preconditions.
        let raw =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        assert!(raw >= 0, "client socket");
        // SAFETY: `raw` is a freshly created descriptor owned by nothing else.
        let connection = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: `sockaddr_un` is a plain C aggregate for which all-zero bytes are valid.
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(target.as_bytes()) {
            *slot = *byte as libc::c_char;
        }
        let deadline = Instant::now() + PATIENCE;
        loop {
            // SAFETY: `address` is fully initialised and its exact size is passed.
            let connected = unsafe {
                libc::connect(
                    connection.as_raw_fd(),
                    (&raw const address).cast(),
                    std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                )
            };
            if connected == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "client never connected");
            thread::sleep(Duration::from_millis(20));
        }
        Self { connection }
    }

    /// Sends one request and returns the one reply it produced.
    #[must_use]
    pub fn call(&self, request: &Request) -> Reply {
        let bytes = request.encode();
        // SAFETY: `bytes` is a valid buffer for its full length.
        let sent = unsafe {
            libc::send(
                self.connection.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        assert_eq!(sent, bytes.len() as isize, "short request");
        let mut frame = [0_u8; MAX_FRAME + 1];
        // SAFETY: `frame` is a valid writable buffer of exactly the passed length.
        let received = unsafe {
            libc::recv(
                self.connection.as_raw_fd(),
                frame.as_mut_ptr().cast(),
                frame.len(),
                0,
            )
        };
        assert!(received > 0, "the daemon closed the connection");
        Reply::decode(&frame[..received.cast_unsigned()]).expect("reply")
    }
}
