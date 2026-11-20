//! What a killed host leaves behind: its socket file, with nothing answering on it.
//!
//! A test models that by abandoning a listener of its own, and closing the descriptor is not
//! enough to do it. Every process spawned anywhere in this binary while the listener exists
//! starts with a copy of the whole descriptor table and keeps that copy until its own `execve`
//! closes it, and other tests here spawn processes all the time. Traced on a 96-thread host with
//! the lib suite in parallel, the listener of a failing case was released by a `sleep` child of
//! another test as that child's `execve` returned, after the case had already connected to it
//! (issue #108). Shutting the socket down acts on the socket, not on one descriptor, so every
//! copy refuses connections from that moment on.

use std::{
    net::Shutdown,
    os::{
        fd::OwnedFd,
        unix::net::{UnixListener, UnixStream},
    },
    process::{Child, Command, Stdio},
};

use super::{ONE, channel, instance};

/// Leaves `listener`'s socket the way a killed host leaves its own: present and silent.
pub(super) fn abandon(listener: UnixListener) {
    // The standard library offers `shutdown` only on a stream; it is the same call on any
    // socket, and a listener that is shut down refuses every later connection.
    UnixStream::from(OwnedFd::from(listener))
        .shutdown(Shutdown::Both)
        .expect("the abandoned socket is shut down");
}

/// Another process holding a copy of a listener's descriptor, for as long as this value lives.
struct Holder(Child);

impl Holder {
    fn of(listener: &UnixListener) -> Self {
        let copy = listener.try_clone().expect("a copy of the listener");
        let child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::from(OwnedFd::from(copy)))
            .stdout(Stdio::null())
            .spawn()
            .expect("a process to hold the copy");
        Self(child)
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The copy a concurrently spawned child keeps until its `execve`, held here on purpose for the
/// whole case, so the case does not depend on when a child of some other test happens to run.
#[test]
fn an_abandoned_socket_stays_silent_while_another_process_still_holds_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let socket = channel::socket_path(directory.path(), &instance(ONE));
    let listener = UnixListener::bind(&socket).expect("a socket to abandon");
    let _holder = Holder::of(&listener);

    abandon(listener);

    assert!(
        channel::connect(directory.path(), &instance(ONE)).is_err(),
        "a host that is gone must not answer through a copy another process still holds"
    );
    assert!(
        !socket.exists(),
        "a socket no process answers on must not survive the lookup that found it"
    );
}
