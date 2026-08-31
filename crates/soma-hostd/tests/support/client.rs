//! The daemon thread a test serves, and the shipped client that talks to it.
//!
//! The client is [`soma_hostd::client::HostClient`] itself rather than a test-only socket, so
//! a test observes the real connect, encode, dispatch, and decode path an adapter uses, and a
//! closed client really closes a connection.

use std::{
    path::Path,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use soma_hostd::{
    client::{ClientError, HostClient},
    daemon,
};

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

/// Connects one client, waiting briefly for a listener that is still binding.
///
/// The wait covers only [`ClientError::Connect`], because that is the one failure a listener
/// which is not ready yet produces; every other refusal is returned at once.
#[must_use]
pub fn connect(socket: &Path) -> HostClient {
    let deadline = Instant::now() + PATIENCE;
    loop {
        match HostClient::connect(socket) {
            Ok(client) => return client,
            Err(ClientError::Connect(_)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("the client never connected: {error}"),
        }
    }
}
