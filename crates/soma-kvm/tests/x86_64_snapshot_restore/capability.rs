//! Live proofs of what a restored Instance can do for the caller that owns its session.
//!
//! The snapshot proofs beside this module answer whether a machine restores, stays independent
//! of its siblings, and returns one command result. These answer the next question: whether the
//! capabilities the guest protocol grew, a filesystem, a command context, a terminal, and a
//! delivered secret, actually work against a real `node:22` guest on real KVM rather than
//! against a loopback of the codec.
//!
//! Every test here restores from the same shared snapshot the rest of the suite uses, so a
//! capability is proved on the production restore path or it is not proved at all.

#[path = "capability/context.rs"]
mod context;

#[path = "capability/directory.rs"]
mod directory;

#[path = "capability/files.rs"]
mod files;

#[path = "capability/refusal.rs"]
mod refusal;

#[path = "capability/secret.rs"]
mod secret;

#[path = "capability/terminal.rs"]
mod terminal;

use soma_guest::TerminalStatus;

use crate::{x86_64_sandbox_boot_session as session, x86_64_snapshot_restore_instance as instance};

/// The directory these proofs work in, made by the first step of every one that needs it.
pub const WORKSPACE: &[u8] = b"/tmp/soma-live";

/// Requires that the restored Instance released every descriptor and thread it opened.
///
/// Each proof asserts this itself rather than trusting the snapshot tests to have, because a
/// capability that leaks a descriptor per request leaks it only when that request is issued.
pub fn assert_no_leak<T>(instance: &instance::Instance<T>) {
    assert_eq!(
        instance.descriptors.1, instance.descriptors.0,
        "the Instance leaked descriptors: {:?}",
        instance.descriptors
    );
    assert_eq!(
        instance.threads.1, instance.threads.0,
        "the Instance leaked threads: {:?}",
        instance.threads
    );
}

/// Requires that one command succeeded and returns what it printed.
///
/// # Panics
///
/// Panics with the status and the standard error a failed command produced.
pub fn succeeded(label: &str, executed: &session::Executed) -> String {
    assert_eq!(
        executed.status,
        TerminalStatus::Exited(0),
        "[{label}] status={:?} stderr={:?}",
        executed.status,
        String::from_utf8_lossy(&executed.stderr)
    );
    String::from_utf8_lossy(&executed.stdout).into_owned()
}

/// One bounded shell step, for the guest-side setup a proof needs before it asserts.
///
/// The arguments are borrowed rather than built here because a bounded command borrows its
/// argument vector, and every script this suite runs is a constant.
#[must_use]
pub fn shell<'a>(arguments: &'a [&'a [u8]]) -> session::Command<'a> {
    session::Command {
        program: b"/bin/sh",
        arguments,
        timeout_millis: 60_000,
        output_bytes: 262_144,
    }
}
