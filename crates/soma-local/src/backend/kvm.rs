mod boot;
mod evidence;
mod identity;
mod io;
mod lifecycle;
mod network;
mod prepared;
mod resolve;
mod runtime;
mod secrets;
mod session;
mod timeline;
mod worker;

use soma::BackendKind;

use crate::{LocalFailure, LocalFailureKind};

use super::clock::OperationClocks;
use network::BrokerConfiguration;
use runtime::Ownership;

pub(crate) struct KvmBackend {
    clocks: OperationClocks,
    /// Who owns the Instances this Backend launches.
    ownership: Ownership,
    /// The one sandbox this Backend is driving, if any.
    live: Option<lifecycle::Live>,
    /// The privileged network broker this host is configured to reach, if it has one.
    ///
    /// This is read once rather than per request: whether a host has a broker is a property of
    /// the host, and a launch that found one and a later launch that did not would otherwise
    /// disagree about what this Backend can serve.
    broker: Option<Box<BrokerConfiguration>>,
}

impl KvmBackend {
    /// Opens the Backend.
    ///
    /// There is nothing to probe. Every artifact a machine needs lives in the store of the
    /// Generation the host prepared, named by digest in its manifest, so a request either finds
    /// a prepared Generation or is refused by name. A Backend that probed the host here would
    /// be asserting something about Generations it has not looked at.
    ///
    /// The one thing that is resolved here is who owns Instances. A configured Host Runtime
    /// that cannot be reached refuses the Backend rather than degrading to the one-shot
    /// lifecycle, because an operator who asked for persistent ownership must not silently
    /// receive an Instance that dies with this process.
    ///
    /// # Errors
    ///
    /// Returns [`LocalFailureKind::BackendUnavailable`] when a Host Runtime is configured and
    /// nothing serves it.
    pub(super) fn open() -> Result<Self, LocalFailure> {
        let ownership = Ownership::resolve(Ownership::configured().as_deref())
            .map_err(|_| LocalFailure::new(LocalFailureKind::BackendUnavailable))?;
        Ok(Self {
            clocks: OperationClocks::new(),
            ownership,
            live: None,
            broker: BrokerConfiguration::from_environment().map(Box::new),
        })
    }

    pub(super) const fn kind() -> BackendKind {
        BackendKind::LinuxKvm
    }
}
