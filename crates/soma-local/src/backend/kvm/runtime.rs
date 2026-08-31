//! Who owns the Instances this Backend launches.
//!
//! A KVM sandbox used to be owned by nothing but the `Option<Live>` of one Backend, so the
//! Instance died with the command-line process that created it and a later `exec` or
//! `destroy` naming that Instance found nothing. The Host Runtime of `soma-hostd` owns
//! Instances across process lifetimes, so this Backend registers with it and afterwards
//! addresses its Instance by identity rather than by the handle it happens to hold.
//!
//! Where no Host Runtime is configured the Backend keeps exactly the one-shot in-process
//! lifecycle it has always had. The two cases are separate variants rather than one path with
//! a fallback inside it, so no operation can quietly report success against a Host that never
//! heard of it.

mod frame;

use std::{
    ffi::{OsStr, OsString},
    path::Path,
};

use soma::{BackendFailureKind, InstanceId, OperationId};
use soma_hostd::{
    FailureCode,
    client::{ClientError, HostClient, Registration},
};

/// The environment locator naming the Host Runtime socket this Backend registers with.
pub(super) const SOCKET_VARIABLE: &str = "SOMA_HOSTD_SOCKET";

/// Who owns the Instances one Backend launches.
pub(super) enum Ownership {
    /// No Host Runtime is configured, so this process owns its Instance for as long as it
    /// runs and no longer, which is the development lifecycle unchanged.
    InProcess,
    /// The Host Runtime behind this connection owns every Instance this Backend launches, so
    /// a later process addresses the same Instance by identity.
    HostRuntime(HostClient),
}

impl Ownership {
    /// Returns the configured locator, or `None` when this Host has no Runtime.
    pub(super) fn configured() -> Option<OsString> {
        std::env::var_os(SOCKET_VARIABLE).filter(|value| !value.is_empty())
    }

    /// Resolves who owns Instances from the locator an operator supplied.
    ///
    /// # Errors
    ///
    /// Returns the client refusal when a locator is configured but nothing serves it. A
    /// configured Runtime that cannot be reached is a failure rather than a reason to fall
    /// back: an operator who asked for persistent ownership and silently received the one-shot
    /// lifecycle would find their Instance gone with nothing having reported a problem.
    pub(super) fn resolve(locator: Option<&OsStr>) -> Result<Self, ClientError> {
        match locator {
            None => Ok(Self::InProcess),
            Some(path) => HostClient::connect(Path::new(path)).map(Self::HostRuntime),
        }
    }

    /// Registers one Instance with the Host Runtime, if there is one.
    ///
    /// Registration happens before the machine is built so that the identity has an owner from
    /// the moment it exists; a Launch that then fails withdraws it through [`Self::withdraw`].
    pub(super) fn register(
        &self,
        instance: &InstanceId,
        operation: &OperationId,
        vsock_cid: u32,
    ) -> Result<(), BackendFailureKind> {
        let Self::HostRuntime(client) = self else {
            return Ok(());
        };
        let frame = frame::launch_frame(instance, operation, vsock_cid)?;
        match client.launch(&frame) {
            Ok(Registration::Live { .. }) => Ok(()),
            // The Host holds a worker for this operation whose launch page it cannot repeat,
            // so this Launch may not proceed under the same identity.
            Ok(Registration::Replayed { .. }) => Err(BackendFailureKind::ResourceConflict),
            Err(error) => Err(launch_refusal(error)),
        }
    }

    /// Withdraws a registration whose machine never reached Ready.
    ///
    /// The failure that is being reported is the one the caller already has, so a withdrawal
    /// that itself fails changes nothing it could act on; what it must not do is leave the
    /// Host owning an Instance no process is serving, which reconciliation would later have to
    /// discover.
    pub(super) fn withdraw(&self, instance: &InstanceId) {
        if let Self::HostRuntime(client) = self
            && let Ok(instance) = frame::host_instance(instance)
        {
            let _ignored = client.destroy(instance);
        }
    }

    /// Ends the Host Runtime's ownership of one Instance.
    ///
    /// Returns whether ownership is proven ended. An Instance no Host Runtime owns is ended by
    /// definition; a receipt the Host could not complete leaves ownership uncertain, which a
    /// caller must not report as a complete cleanup.
    ///
    /// # Errors
    ///
    /// Returns [`BackendFailureKind::CleanupFailure`] when the Host refused the terminal
    /// operation, because a Machine record that no client has ended is a leak.
    pub(super) fn release(&self, instance: &InstanceId) -> Result<bool, BackendFailureKind> {
        let Self::HostRuntime(client) = self else {
            return Ok(true);
        };
        let identity = frame::host_instance(instance)?;
        match client.destroy(identity) {
            Ok(receipt) => Ok(receipt.complete),
            // An Instance no durable record carries was never owned, so there is nothing left
            // to end; every other refusal leaves ownership unresolved and is reported.
            Err(ClientError::Refused(FailureCode::Unknown)) => Ok(true),
            Err(_) => Err(BackendFailureKind::CleanupFailure),
        }
    }

    /// Whether the Host Runtime still reports this Instance as live.
    pub(super) fn is_live(&self, instance: &InstanceId) -> bool {
        let Self::HostRuntime(client) = self else {
            return false;
        };
        frame::host_instance(instance).is_ok_and(|identity| client.get(identity).is_ok())
    }
}

/// The failure kind one refused registration reports.
///
/// A refusal by the Host is about the Host or the request, never about the guest, so none of
/// these may become a guest failure: an operator reading `GuestFailure` would look inside a
/// machine that was never built.
const fn launch_refusal(error: ClientError) -> BackendFailureKind {
    match error {
        ClientError::Refused(FailureCode::Conflict | FailureCode::Terminated) => {
            BackendFailureKind::ResourceConflict
        }
        ClientError::Refused(FailureCode::Capacity | FailureCode::Exhausted) => {
            BackendFailureKind::WorkloadRejected
        }
        _ => BackendFailureKind::Unavailable,
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
