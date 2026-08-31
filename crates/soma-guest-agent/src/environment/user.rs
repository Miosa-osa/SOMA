//! Resolving the user name a command asks for to the numeric identity `execve` needs.
//!
//! The account database is read directly rather than through `getpwnam`. The agent is one
//! statically linked binary with no name-service plugins to load, so the library call could only
//! consult the same file this does, and reading it here keeps the lookup free of C library state
//! and testable without a guest.

use std::fs;

/// The account database every image this agent boots is expected to carry.
const PASSWD: &str = "/etc/passwd";

/// The numeric identity one named account resolves to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Credentials {
    /// The account's user identifier.
    pub uid: u32,
    /// The account's primary group identifier.
    pub gid: u32,
}

/// Resolves one account name against the guest's own database.
///
/// Returns `None` when the database cannot be read or holds no such account, which the caller
/// reports as a refused invocation rather than silently running as somebody else.
pub fn resolve(name: &[u8]) -> Option<Credentials> {
    lookup(&fs::read(PASSWD).ok()?, name)
}

/// Finds one account in the exact bytes of a `passwd` database.
///
/// The fields are `name:password:uid:gid:...`, colon separated, one account to a line. A line
/// with too few fields or an unparsable identifier is skipped rather than failing the lookup,
/// because one malformed line in an image's database must not decide who a command runs as.
pub fn lookup(passwd: &[u8], name: &[u8]) -> Option<Credentials> {
    passwd
        .split(|byte| *byte == b'\n')
        .find_map(|line| account(line, name))
}

fn account(line: &[u8], name: &[u8]) -> Option<Credentials> {
    let mut fields = line.split(|byte| *byte == b':');
    if fields.next()? != name {
        return None;
    }
    let mut fields = fields.skip(1);
    let uid = number(fields.next()?)?;
    let gid = number(fields.next()?)?;
    Some(Credentials { uid, gid })
}

fn number(field: &[u8]) -> Option<u32> {
    core::str::from_utf8(field).ok()?.parse().ok()
}
