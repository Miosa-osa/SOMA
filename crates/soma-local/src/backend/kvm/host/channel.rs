//! Where a machine host listens, and the one line at a time its callers exchange with it.
//!
//! A host is addressed by the Instance it holds and by nothing else: its socket is named by the
//! Instance identity, so resolving an Instance to the process serving it is a path lookup rather
//! than a registry that could disagree with the processes that exist.

use std::{
    fs,
    io::{BufRead, Read as _, Write},
    os::unix::{
        fs::PermissionsExt as _,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use soma::InstanceId;

use super::wire::MAX_LINE_BYTES;

/// Only the owning user may address a machine host.
const PRIVATE: u32 = 0o700;

/// How many bytes of path a Unix domain socket address holds, terminator included.
///
/// `sockaddr_un.sun_path` on Linux. A `bind` past it fails with the same error as a missing
/// directory, which is why a state root one byte too deep reads as a host that would not start
/// rather than as a state root that cannot address one.
const MAX_SOCKET_PATH_BYTES: usize = 108;

pub(super) fn socket_path(directory: &Path, instance: &InstanceId) -> PathBuf {
    directory.join(format!("{}.sock", instance.as_str()))
}

/// Whether a machine host under `directory` can be addressed at all.
///
/// Every Instance identity is the same length, so this is a property of the directory rather
/// than of the request, and it is answered before a process is spawned. A hundred launches once
/// failed here as `backend_unavailable` with nothing said, because the state root was thirty-six
/// bytes deep and the socket name needs seventy-two.
pub(super) fn addressable(directory: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    // 32 identity characters, `.sock`, the separator the join adds, and the terminator.
    let name = InstanceId::LENGTH + ".sock".len() + 1 + 1;
    directory.as_os_str().as_bytes().len() + name <= MAX_SOCKET_PATH_BYTES
}

/// Creates the socket directory, readable and writable only by its owner.
pub(super) fn prepare_directory(directory: &Path) -> Result<(), ()> {
    fs::create_dir_all(directory).map_err(|_| ())?;
    fs::set_permissions(directory, fs::Permissions::from_mode(PRIVATE)).map_err(|_| ())
}

/// Binds the host's socket, refusing rather than replacing one something already answers on.
pub(super) fn bind(socket: &Path) -> Result<UnixListener, ()> {
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Err(());
        }
        // Nothing answers, so the file is the remains of a host that is gone.
        fs::remove_file(socket).map_err(|_| ())?;
    }
    let listener = UnixListener::bind(socket).map_err(|_| ())?;
    fs::set_permissions(socket, fs::Permissions::from_mode(PRIVATE)).map_err(|_| ())?;
    Ok(listener)
}

/// Connects to the host serving `instance`, clearing a socket nothing answers on.
///
/// A failure here is always absence rather than a broken exchange: either no socket exists or
/// no process is behind the one that does, and both mean nothing here serves this Instance.
pub(super) fn connect(directory: &Path, instance: &InstanceId) -> Result<UnixStream, ()> {
    let socket = socket_path(directory, instance);
    if let Ok(stream) = UnixStream::connect(&socket) {
        return Ok(stream);
    }
    // A socket no process answers on names an Instance nothing is serving, and leaving it behind
    // would make every later lookup report a host that is not there.
    let _ignored = fs::remove_file(&socket);
    Err(())
}

pub(super) fn write_line(writer: &mut impl Write, value: &impl Serialize) -> Result<(), ()> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| ())?;
    bytes.push(b'\n');
    writer.write_all(&bytes).map_err(|_| ())?;
    writer.flush().map_err(|_| ())
}

/// Reads one bounded line, returning nothing when the peer is gone or the line is inadmissible.
pub(super) fn read_line<T: DeserializeOwned>(reader: &mut impl BufRead) -> Option<T> {
    let mut line = Vec::new();
    let mut bounded = reader.take(MAX_LINE_BYTES);
    bounded.read_until(b'\n', &mut line).ok()?;
    if line.is_empty() {
        return None;
    }
    serde_json::from_slice(&line).ok()
}
