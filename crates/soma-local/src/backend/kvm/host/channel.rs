//! Where a machine host listens, and the one line at a time its callers exchange with it.
//!
//! A host is addressed by the Instance it holds and nothing else: the socket is named by the
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

/// The directory holding one socket per hosted Instance.
pub(in crate::backend::kvm) fn directory(state_root: &Path) -> PathBuf {
    state_root.join("machines")
}

pub(super) fn socket_path(directory: &Path, instance: &InstanceId) -> PathBuf {
    directory.join(format!("{}.sock", instance.as_str()))
}

/// Creates the socket directory, readable and writable only by its owner.
pub(super) fn prepare_directory(directory: &Path) -> Result<(), ()> {
    fs::create_dir_all(directory).map_err(|_| ())?;
    fs::set_permissions(directory, fs::Permissions::from_mode(PRIVATE)).map_err(|_| ())
}

/// Binds the host's socket, refusing rather than replacing a socket something already serves.
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

/// Why a caller could not reach a machine host.
pub(super) enum Unreachable {
    /// No host serves this Instance here.
    Absent,
    /// A host is there and the exchange with it did not complete.
    Broken,
}

/// Connects to the host serving `instance`, clearing a socket nothing answers on.
pub(super) fn connect(directory: &Path, instance: &InstanceId) -> Result<UnixStream, Unreachable> {
    let socket = socket_path(directory, instance);
    match UnixStream::connect(&socket) {
        Ok(stream) => Ok(stream),
        Err(_) => {
            // A socket no process answers on names an Instance nothing is serving, and leaving
            // it behind would make every later lookup report a host that is not there.
            let _ignored = fs::remove_file(&socket);
            Err(Unreachable::Absent)
        }
    }
}

pub(super) fn write_line(writer: &mut impl Write, value: &impl Serialize) -> Result<(), ()> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| ())?;
    bytes.push(b'\n');
    writer.write_all(&bytes).map_err(|_| ())?;
    writer.flush().map_err(|_| ())
}

/// Reads one bounded line, returning nothing when the peer is gone or the line is not admissible.
pub(super) fn read_line<T: DeserializeOwned>(reader: &mut impl BufRead) -> Option<T> {
    let mut line = Vec::new();
    let mut bounded = reader.take(MAX_LINE_BYTES);
    bounded.read_until(b'\n', &mut line).ok()?;
    if line.is_empty() {
        return None;
    }
    serde_json::from_slice(&line).ok()
}
