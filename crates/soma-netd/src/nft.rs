//! Version 1 nftables and conntrack mechanism: pinned binaries fed structured text.
//!
//! The broker never composes shell; it executes `/usr/sbin/nft -f -` with the generated
//! ruleset on standard input inside the calling thread's namespace, and
//! `/usr/sbin/conntrack -D -w <zone>` in the host namespace.
//!
//! Presence of one table is asked of that table alone; only reconciliation, which must find
//! tables no ledger record names, lists the whole namespace.
//!
//! Every invocation is contained by [`soma_supervise`], which the Generation compiler already
//! uses for its build tools: the tool leads its own process group, an absolute deadline bounds
//! the wait, the retained output is bounded before it is allocated, and a timeout, a failed
//! ruleset write, a capture overflow, or a descendant holding the pipes terminates the whole
//! group and reports a typed [`Error::Tool`].
//! The single-threaded broker therefore always regains control, so release and reconciliation
//! stay available even when a tool wedges.
//!
//! A libnftnl binding replaces this seam later without changing the ruleset generators.

use std::{process::Command, time::Duration};

use soma_supervise::{Contained, Output, Uncontained};

use crate::{ConntrackZone, Error, Tool};

const NFT: &str = "/usr/sbin/nft";
const CONNTRACK: &str = "/usr/sbin/conntrack";

/// Absolute deadline for one privileged tool invocation.
///
/// A ruleset transaction or a conntrack zone flush is a bounded kernel operation; anything
/// slower is a wedged tool rather than a slow one, and cleanup must not wait for it.
const DEADLINE: Duration = Duration::from_secs(10);

/// One bounded invocation of one privileged tool.
struct Invocation<'a> {
    tool: Tool,
    program: &'a str,
    arguments: &'a [&'a str],
    deadline: Duration,
}

impl Invocation<'_> {
    /// Runs the tool with `input` on standard input and returns its bounded output.
    ///
    /// A write that the tool refuses is the operation's failure, not a discarded detail.
    fn run(&self, input: &str) -> Result<Output, Error> {
        let mut command = Command::new(self.program);
        command.args(self.arguments);
        let refused = Error::Tool {
            tool: self.tool,
            status: None,
        };
        Contained::new(command, self.deadline)
            .run(|stdin| stdin.write_all(input.as_bytes()).map_err(|_| refused))
            .map_err(|failure| match failure {
                Uncontained::Input(error) => error,
                Uncontained::Spawn | Uncontained::Terminated | Uncontained::Lost => Error::Tool {
                    tool: self.tool,
                    status: None,
                },
            })
    }
}

fn nft<'a>(arguments: &'a [&'a str]) -> Invocation<'a> {
    Invocation {
        tool: Tool::Nft,
        program: NFT,
        arguments,
        deadline: DEADLINE,
    }
}

/// Applies one complete ruleset text.
pub(crate) fn apply(ruleset: &str) -> Result<(), Error> {
    let output = nft(&["-f", "-"]).run(ruleset)?;
    if output.succeeded() {
        Ok(())
    } else {
        Err(Error::Tool {
            tool: Tool::Nft,
            status: output.exit_code,
        })
    }
}

/// Lists the `inet` table names of the calling thread's namespace.
pub(crate) fn list_tables() -> Result<Vec<String>, Error> {
    let output = nft(&["list", "tables", "inet"]).run("")?;
    if !output.succeeded() {
        return Err(Error::Tool {
            tool: Tool::Nft,
            status: output.exit_code,
        });
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut names: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("table inet "))
        .map(|name| name.trim().to_owned())
        .collect();
    names.sort_unstable();
    Ok(names)
}

/// Reports whether one `inet` table exists in the calling thread's namespace.
///
/// The question is asked of that one table rather than of the whole namespace, so the answer
/// stays bounded no matter how many tables the namespace holds and cleanup can never be
/// disabled by an accumulation of leaked tables.
pub(crate) fn table_exists(name: &str) -> Result<bool, Error> {
    table_presence(&nft(&["list", "table", "inet", name]).run("")?)
}

/// The exact presence decision, separated from the invocation that produced the output.
fn table_presence(output: &Output) -> Result<bool, Error> {
    if output.succeeded() {
        return Ok(true);
    }
    if String::from_utf8_lossy(&output.stderr).contains("No such file or directory") {
        return Ok(false);
    }
    Err(Error::Tool {
        tool: Tool::Nft,
        status: output.exit_code,
    })
}

/// Deletes one `inet` table; returns `false` when it was already absent.
pub(crate) fn delete_table(name: &str) -> Result<bool, Error> {
    if !table_exists(name)? {
        return Ok(false);
    }
    apply(&format!("delete table inet {name}\n")).map(|()| true)
}

/// Flushes every conntrack entry of one zone in the calling thread's namespace.
pub(crate) fn flush_zone(zone: ConntrackZone) -> Result<(), Error> {
    let zone = zone.get().to_string();
    let output = Invocation {
        tool: Tool::Conntrack,
        program: CONNTRACK,
        arguments: &["-D", "-w", &zone],
        deadline: DEADLINE,
    }
    .run("")?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.succeeded() || stderr.contains("0 flow entries have been deleted") {
        Ok(())
    } else {
        Err(Error::Tool {
            tool: Tool::Conntrack,
            status: output.exit_code,
        })
    }
}

#[cfg(test)]
mod tests;
