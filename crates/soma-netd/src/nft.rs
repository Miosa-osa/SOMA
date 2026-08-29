//! Version 1 nftables and conntrack mechanism: pinned binaries fed structured text.
//!
//! The broker never composes shell; it executes `/usr/sbin/nft -f -` with the generated
//! ruleset on standard input inside the calling thread's namespace, and
//! `/usr/sbin/conntrack -D -w <zone>` in the host namespace.
//! A libnftnl binding replaces this seam later without changing the ruleset generators.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use crate::{ConntrackZone, Error, Tool};

const NFT: &str = "/usr/sbin/nft";
const CONNTRACK: &str = "/usr/sbin/conntrack";

/// Applies one complete ruleset text.
pub(crate) fn apply(ruleset: &str) -> Result<(), Error> {
    let mut child = Command::new(NFT)
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| Error::Tool {
            tool: Tool::Nft,
            status: None,
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(ruleset.as_bytes());
    }
    let status = child.wait().map_err(|_| Error::Tool {
        tool: Tool::Nft,
        status: None,
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Tool {
            tool: Tool::Nft,
            status: status.code(),
        })
    }
}

/// Lists the `inet` table names of the calling thread's namespace.
pub(crate) fn list_tables() -> Result<Vec<String>, Error> {
    let output = Command::new(NFT)
        .args(["list", "tables", "inet"])
        .stderr(Stdio::null())
        .output()
        .map_err(|_| Error::Tool {
            tool: Tool::Nft,
            status: None,
        })?;
    if !output.status.success() {
        return Err(Error::Tool {
            tool: Tool::Nft,
            status: output.status.code(),
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

/// Deletes one `inet` table; returns `false` when it was already absent.
pub(crate) fn delete_table(name: &str) -> Result<bool, Error> {
    if !list_tables()?.iter().any(|table| table == name) {
        return Ok(false);
    }
    apply(&format!("delete table inet {name}\n")).map(|()| true)
}

/// Flushes every conntrack entry of one zone in the calling thread's namespace.
pub(crate) fn flush_zone(zone: ConntrackZone) -> Result<(), Error> {
    let output = Command::new(CONNTRACK)
        .args(["-D", "-w", &zone.get().to_string()])
        .stdout(Stdio::null())
        .output()
        .map_err(|_| Error::Tool {
            tool: Tool::Conntrack,
            status: None,
        })?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() || stderr.contains("0 flow entries have been deleted") {
        Ok(())
    } else {
        Err(Error::Tool {
            tool: Tool::Conntrack,
            status: output.status.code(),
        })
    }
}
