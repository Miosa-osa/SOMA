//! Process environment policy for direct command execution.
//!
//! The agent owns a base environment and a default working directory, and the caller may add to
//! the first and replace the second through the command's own context. The base is not a
//! ceiling the caller may raise: it is what a program finds when the caller says nothing, so a
//! command that names no `PATH` still finds one, and a command that names its own gets it.
//!
//! Nothing here is inherited from the agent's own process. The environment is cleared before
//! the base is applied, so what a program sees is exactly what this module assembled.

mod invocation;
mod user;

#[cfg(test)]
mod tests;

pub use invocation::{InvalidInvocation, Invocation};

use soma_guest::EnvironmentPair;

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;

/// Working directory for a command that does not name one.
pub const WORKING_DIRECTORY: &str = "/";

/// The environment every direct command starts from.
pub const ENVIRONMENT: &[(&str, &str)] = &[
    ("HOME", "/root"),
    ("LANG", "C.UTF-8"),
    (
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    ),
    ("SOMA_SANDBOX", "1"),
    ("TERM", "dumb"),
];

/// Maximum number of arguments the agent will pass to `execve`.
pub const MAX_ARGUMENTS: usize = 64;
/// Maximum byte length of the program path, one argument, or one environment name or value.
pub const MAX_FIELD_BYTES: usize = 4096;
/// Maximum number of environment variables the agent will accept from one command.
pub const MAX_ENVIRONMENT: usize = 64;

/// Applies the caller's pairs over the base environment, in the caller's order.
///
/// A caller pair replaces a base pair of the same name in place rather than being appended,
/// because `Command::envs` would otherwise carry both and leave which one wins to the order the
/// standard library happens to apply them in.
fn merge(caller: &[EnvironmentPair]) -> Vec<(OsString, OsString)> {
    let mut merged: Vec<(OsString, OsString)> = ENVIRONMENT
        .iter()
        .map(|(name, value)| (OsString::from(*name), OsString::from(*value)))
        .collect();
    for (name, value) in caller {
        let name = OsString::from_vec(name.to_vec());
        let value = OsString::from_vec(value.to_vec());
        match merged.iter_mut().find(|(existing, _)| *existing == name) {
            Some(slot) => slot.1 = value,
            None => merged.push((name, value)),
        }
    }
    merged
}
