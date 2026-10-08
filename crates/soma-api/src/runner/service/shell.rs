//! How one exec string reaches the guest: through a login shell, a plain shell, or as its own
//! argv with no shell at all (plan track T1).
//!
//! Contract C2 carries a command as one string, the way the fast lane's host executor took it,
//! and that executor ran every string through `/bin/sh -lc`. Two costs come with that for a
//! command that is nothing but whitespace-separated words: the login shell sources the profile
//! files on every single command, and a shell process stands between the agent and the program.
//! The guest agent already runs a direct `execve` with no shell anywhere in the path, so a
//! command that needs no interpretation can go straight there.
//!
//! The classifier is deliberately conservative. Anything it cannot prove is a plain word list
//! keeps a shell, byte-for-byte as before, so this decision never changes what a command means.
//! It only chooses the cheaper route for the commands it fully understands.

/// Characters a POSIX shell treats as syntax. Any one of them means the string needs the shell
/// to be interpreted, so it is never sent as argv.
///
/// `=` covers the assignment prefix (`FOO=bar cmd`) and an option spelled `--name=value`, and
/// `~` covers tilde expansion; both are things the shell would rewrite and `execve` would not.
const SHELL_SYNTAX: &[char] = &[
    '|', '&', ';', '<', '>', '(', ')', '$', '`', '\\', '"', '\'', '*', '?', '[', ']', '#', '~',
    '=', '%', '{', '}', '!',
];

/// Words a shell defines for itself. Running one of these as a program is not the same act as
/// running the shell's own, and most have no external program at all, so a command whose first
/// word is one of these keeps the shell.
///
/// This is the POSIX special and regular builtin list plus the common extensions, whether or not
/// the image ships a binary of the same name: `echo`, `printf`, `test`, `pwd`, `kill`, `true`
/// and `false` all exist as binaries, but their argument handling belongs to the shell and is
/// not specified to match the binary's.
const SHELL_BUILTINS: &[&str] = &[
    ":",
    ".",
    "[",
    "alias",
    "bg",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "jobs",
    "kill",
    "let",
    "local",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "select",
    "set",
    "shift",
    "source",
    "test",
    "time",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
];

/// The program that resolves a bare command name, so a word like `node` reaches its absolute
/// path without a shell. `env` replaces itself with that program, so it adds no second process.
///
/// Every generation the runner launches is OCI-derived, and the runner's own launch image
/// (`docker.io/library/node:22`) carries coreutils: `/usr/bin/env` is present in the image and
/// `/bin` is a symlink to `/usr/bin`. The path is absolute so the agent's own `PATH` never has
/// to find it.
const ENV: &str = "/usr/bin/env";

/// The shell the fast lane used, kept for the no-profile variant and for the rollback.
pub(super) const SHELL: &str = "/bin/sh";

/// One exec string's route to the guest.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Plan<'a> {
    /// `/bin/sh -lc <command>`: exactly the fast lane's argv, and what one flag restores.
    LoginShell(&'a str),
    /// `/bin/sh -c <command>`: the shell without sourcing a profile of its own.
    PlainShell(&'a str),
    /// The command's own words, run with no shell.
    Direct {
        program: &'a str,
        arguments: Vec<&'a str>,
    },
}

/// Chooses how `command` reaches the guest.
///
/// With `shell_free_exec` off this is the login shell exactly as it was before the change, so an
/// operator who meets a command this misroutes restores the old behavior with one config edit.
pub(super) fn plan(command: &str, shell_free_exec: bool) -> Plan<'_> {
    if !shell_free_exec {
        return Plan::LoginShell(command);
    }
    match argv(command) {
        Some((program, arguments)) => Plan::Direct { program, arguments },
        None => Plan::PlainShell(command),
    }
}

/// The argv a command runs as when it needs no shell, or `None` when a shell must interpret it.
fn argv(command: &str) -> Option<(&str, Vec<&str>)> {
    if command
        .chars()
        .any(|character| SHELL_SYNTAX.contains(&character))
    {
        return None;
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    let (program, arguments) = words.split_first()?;
    // A leading dash is an option to whatever runs the command, never a program name, and the
    // lookup program would read it as one of its own options.
    if program.starts_with('-') || SHELL_BUILTINS.contains(program) {
        return None;
    }
    if program.starts_with('/') {
        return Some((program, arguments.to_vec()));
    }
    // A bare name: the agent execs an absolute path only, so the lookup program resolves the
    // name exactly as the shell would have, then replaces itself with it.
    Some((ENV, words))
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
