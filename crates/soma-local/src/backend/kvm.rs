mod boot;
mod evidence;
mod io;
mod lifecycle;
mod prepared;
mod resolve;
mod session;
mod worker;

use soma::BackendKind;

use super::clock::OperationClocks;

pub(crate) struct KvmBackend {
    clocks: OperationClocks,
    /// The one sandbox this Backend is driving, if any.
    live: Option<lifecycle::Live>,
}

impl KvmBackend {
    /// Opens the Backend.
    ///
    /// There is nothing to probe. Every artifact a machine needs lives in the store of the
    /// Generation the host prepared, named by digest in its manifest, so a request either finds
    /// a prepared Generation or is refused by name. A Backend that probed the host here would
    /// be asserting something about Generations it has not looked at.
    pub(super) fn open() -> Self {
        Self {
            clocks: OperationClocks::new(),
            live: None,
        }
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::LinuxKvm
    }
}
