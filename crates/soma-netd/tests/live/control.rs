//! A client for the privileged control socket and the broker thread the live authorization
//! proof drives.
//!
//! The client speaks the exact `SOCK_SEQPACKET` frame protocol a jailed host process would
//! speak, so the test observes the daemon's real accept, authentication, and capability path.

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
    thread,
    time::{Duration, Instant},
};

use soma_netd::{ControlAuthority, MAX_FRAME, Reply, Request, serve};

const PATIENCE: Duration = Duration::from_secs(5);

/// Returns the primary group of this process, which must own the control socket.
#[must_use]
pub fn current_group() -> u32 {
    // SAFETY: `getgid` reads this process's identity and has no preconditions.
    unsafe { libc::getgid() }
}

/// Starts one broker on its own socket and returns once that socket accepts connections.
///
/// No bundle is prepared, so the authorization proof creates and leaks no kernel object.
pub fn broker_on(state: &Path, socket: &Path, authority: ControlAuthority) {
    let state = state.to_path_buf();
    let path = socket.to_path_buf();
    thread::spawn(move || {
        let broker = super::broker(&state, 16);
        let _ = serve(broker, &path, 0, authority);
    });
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline && !socket.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(socket.exists(), "the broker never bound its socket");
}

/// One connected control client.
pub struct Client {
    connection: OwnedFd,
}

impl Client {
    /// Connects to the control socket, waiting briefly for the listener to be ready.
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

    /// Sends one request frame.
    pub fn send(&self, request: &Request) {
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
    }

    /// Receives one reply frame, or `None` when the broker closed the connection.
    #[must_use]
    pub fn reply(&self) -> Option<Reply> {
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
        if received <= 0 {
            return None;
        }
        Some(Reply::decode(&frame[..received.cast_unsigned()]).expect("reply"))
    }
}
