//! What a failure to open the local runtime, or to launch on a backend that cannot host, reports.
//!
//! These two are apart from the rest because neither describes an operation that reached a
//! sandbox. One is the runtime failing to open at all and the other is a capability the selected
//! backend does not have, so neither carries a receipt and neither names an Instance.

use soma_local::LocalFailureKind;

use crate::{exit::ProcessExit, model::FailureBody, model::Response};

use super::super::Execution;
use super::state_store_retryable;

pub(in crate::app) fn local_failure(command: &'static str, kind: LocalFailureKind) -> Execution {
    let (body, exit) = match kind {
        LocalFailureKind::InvalidConfiguration => (
            FailureBody::new(
                "invalid_configuration",
                "local runtime configuration is invalid",
                false,
            ),
            ProcessExit::InvalidInput,
        ),
        LocalFailureKind::UnsupportedTarget => (
            FailureBody::new(
                "unsupported_backend",
                "local backend is unsupported on this host",
                false,
            ),
            ProcessExit::UnsupportedBackend,
        ),
        LocalFailureKind::BackendUnavailable => (
            FailureBody::new(
                "backend_unavailable",
                "local isolation backend is unavailable",
                false,
            ),
            ProcessExit::CapabilityUnavailable,
        ),
        LocalFailureKind::StateStore(kind) => (
            FailureBody::new(
                "state_store_failure",
                "durable state store could not be opened",
                state_store_retryable(kind),
            ),
            ProcessExit::Software,
        ),
    };
    Execution {
        response: Response::failure(command, body),
        exit,
    }
}

/// What a launch reports on a backend that cannot host a Machine past this process.
///
/// `CapabilityUnavailable` rather than a backend failure: nothing failed, and nothing was
/// started. The capability the `machine` surface is built on does not exist on this backend yet,
/// and `soma run` is the one that does.
pub(in crate::app) fn not_hosted(command: &'static str) -> Execution {
    Execution {
        response: Response::failure(
            command,
            FailureBody::new(
                "machine_not_hosted",
                "this backend hosts a machine only inside the launching process, so a launched \
                 instance identity would not survive this command; use `soma run` for a \
                 single-process sandbox",
                false,
            ),
        ),
        exit: ProcessExit::CapabilityUnavailable,
    }
}
