//! Process-group containment for one external tool.
//!
//! Every tool is started as the leader of a fresh process group, so one signal reaches the
//! tool and every descendant it forked rather than only the direct child.
//! Signalling a group has no safe standard-library equivalent, so this module owns the crate's
//! only `unsafe` call; nothing else here may signal a process.
//! On platforms without process groups the shims are inert, which is correct because the
//! privileged Linux tools these callers run cannot run there at all.

#![allow(unsafe_code)]

use std::process::Command;

/// Reserved group identifiers that are never signalled.
const LOWEST_SIGNALLABLE_GROUP: u32 = 2;

/// One tool's process group.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Group(u32);

/// Which signal one termination step sends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Signal {
    /// The polite request a well-behaved tool honors.
    Terminate,
    /// The signal a tool cannot catch, block, or ignore.
    Force,
}

impl Group {
    /// The group identifier a freshly spawned isolated child leads.
    ///
    /// [`isolate`] makes the child a group leader, so the group identifier equals its process
    /// identifier.
    pub(crate) const fn new(child_id: u32) -> Self {
        Self(child_id)
    }

    /// Sends one signal to every member of the group.
    ///
    /// The caller must not signal after it has reaped the leader and every other member has
    /// exited, because the identifier is only reserved while the group still has a member.
    #[cfg(unix)]
    pub(crate) fn signal(self, signal: Signal) {
        if self.0 < LOWEST_SIGNALLABLE_GROUP {
            return;
        }
        let Ok(group) = i32::try_from(self.0) else {
            return;
        };
        let number = match signal {
            Signal::Terminate => libc::SIGTERM,
            Signal::Force => libc::SIGKILL,
        };
        // SAFETY: `kill` has no memory preconditions. The negative identifier selects one
        // process group, which is above the reserved identifiers and was created by `isolate`
        // for this invocation, so the call can only reach the tool and its descendants.
        unsafe { libc::kill(-group, number) };
    }

    #[cfg(not(unix))]
    pub(crate) fn signal(self, _signal: Signal) {
        let _ = (self.0, LOWEST_SIGNALLABLE_GROUP);
    }
}

/// Starts the tool as the leader of a fresh process group.
#[cfg(unix)]
pub(crate) fn isolate(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(not(unix))]
pub(crate) fn isolate(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::{Group, Signal};

    #[test]
    fn reserved_group_identifiers_are_never_signalled() {
        Group::new(0).signal(Signal::Terminate);
        Group::new(1).signal(Signal::Force);
        assert_eq!(Group::new(7), Group::new(7));
        assert_ne!(Group::new(7), Group::new(8));
    }
}
