//! Which shell one exec string runs under (plan track T1).
//!
//! Contract C2 carries a command as one string, the way the fast lane's host executor took it,
//! and that executor ran every string through `/bin/sh -lc`. The `-l` makes the shell a login
//! shell, so it sources the profile files on every single command, which no exec needs. A
//! command that names no profile-only tool runs the same under a plain shell for less.

/// The shell the fast lane used, kept for both shells and for the rollback.
pub(super) const SHELL: &str = "/bin/sh";

/// One exec string's route to the guest.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Plan<'a> {
    /// `/bin/sh -lc <command>`: exactly the fast lane's argv, and what one flag restores.
    LoginShell(&'a str),
    /// `/bin/sh -c <command>`: the shell without sourcing a profile of its own.
    PlainShell(&'a str),
}

/// Chooses how `command` reaches the guest.
///
/// With `shell_free_exec` off this is the login shell exactly as it was before the change, so an
/// operator who meets a command this misroutes restores the old behavior with one config edit.
pub(super) fn plan(command: &str, shell_free_exec: bool) -> Plan<'_> {
    if shell_free_exec {
        Plan::PlainShell(command)
    } else {
        Plan::LoginShell(command)
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
