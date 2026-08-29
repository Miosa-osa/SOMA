//! Audit of a head directory against the ownership ledger.
//!
//! Reconciliation only reports; it never unlinks, because a head that the ledger does not
//! know may belong to a crashed owner whose evidence has not been replayed yet.

use std::fmt;
use std::io;
use std::os::fd::BorrowedFd;

use crate::head::{HeadName, HeadToken};
use crate::lease::HeadLedger;
use crate::release::reopen;

/// Typed finding for one directory entry or ledger assignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// The entry is a regular file owned by exactly one current assignment.
    Consistent {
        /// Owning token.
        token: HeadToken,
        /// Head name.
        name: HeadName,
    },
    /// The entry is a well-formed head name that no current assignment owns.
    Orphan {
        /// Head name.
        name: HeadName,
        /// True when the ledger retired the name earlier, so a release did not finish.
        retired: bool,
    },
    /// The ledger assigns a head that does not exist in the directory.
    Missing {
        /// Owning token.
        token: HeadToken,
        /// Head name.
        name: HeadName,
    },
    /// The entry is not a head: a foreign name, a non-regular file, or an unreadable entry.
    Foreign {
        /// Entry name as reported by the directory, lossily decoded.
        entry: String,
        /// Why it is not a head.
        reason: ForeignReason,
    },
}

/// Why a directory entry is not treated as a head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignReason {
    /// The name violates the head-name rules.
    InvalidName,
    /// The entry is a directory, symbolic link, or other non-regular file.
    NotRegularFile,
    /// The entry metadata could not be read.
    Unreadable,
}

impl fmt::Display for ForeignReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => f.write_str("invalid head name"),
            Self::NotRegularFile => f.write_str("not a regular file"),
            Self::Unreadable => f.write_str("unreadable entry"),
        }
    }
}

/// Complete report of one reconciliation pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    dispositions: Vec<Disposition>,
}

impl ReconcileReport {
    /// Every finding in directory order followed by missing assignments in token order.
    #[must_use]
    pub fn dispositions(&self) -> &[Disposition] {
        &self.dispositions
    }

    /// True when every entry is consistent and no assignment is missing.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.dispositions
            .iter()
            .all(|d| matches!(d, Disposition::Consistent { .. }))
    }

    /// Number of findings of one kind.
    #[must_use]
    pub fn count(&self, predicate: impl Fn(&Disposition) -> bool) -> usize {
        self.dispositions.iter().filter(|d| predicate(d)).count()
    }
}

/// Scans `dir` and compares every entry with `ledger`.
///
/// # Errors
///
/// Propagates a failure to reopen or read the directory; per-entry failures become
/// [`Disposition::Foreign`] findings.
pub fn reconcile(ledger: &HeadLedger, dir: BorrowedFd<'_>) -> io::Result<ReconcileReport> {
    let handle = reopen(dir)?;
    let mut dispositions = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for entry in handle.entries()? {
        let entry = entry?;
        let raw = entry.file_name();
        let entry_text = raw.to_string_lossy().into_owned();
        let Ok(name) = HeadName::new(entry_text.clone()) else {
            dispositions.push(Disposition::Foreign {
                entry: entry_text,
                reason: ForeignReason::InvalidName,
            });
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            dispositions.push(Disposition::Foreign {
                entry: entry_text,
                reason: ForeignReason::Unreadable,
            });
            continue;
        };
        if !kind.is_file() {
            dispositions.push(Disposition::Foreign {
                entry: entry_text,
                reason: ForeignReason::NotRegularFile,
            });
            continue;
        }
        seen.insert(name.clone());
        if let Some(token) = ledger.owner(&name) {
            dispositions.push(Disposition::Consistent { token, name });
        } else {
            let retired = ledger.is_retired_name(&name);
            dispositions.push(Disposition::Orphan { name, retired });
        }
    }
    for (token, name) in ledger.assignments() {
        if !seen.contains(name) {
            dispositions.push(Disposition::Missing {
                token,
                name: name.clone(),
            });
        }
    }
    Ok(ReconcileReport { dispositions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::fd::AsFd;

    fn token(byte: u8) -> HeadToken {
        HeadToken::new([byte; 16]).expect("non-zero")
    }

    #[test]
    fn reports_consistent_orphan_missing_and_foreign_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut ledger = HeadLedger::new();
        let owned = token(1).head_name();
        let missing = token(2).head_name();
        let orphan = token(3).head_name();
        let retired = token(4).head_name();
        ledger.lease(token(1), owned.clone()).expect("lease");
        ledger.lease(token(2), missing.clone()).expect("lease");
        ledger.lease(token(4), retired.clone()).expect("lease");
        ledger.release(token(4)).expect("release");
        std::fs::write(temp.path().join(owned.as_str()), b"a").expect("write");
        std::fs::write(temp.path().join(orphan.as_str()), b"b").expect("write");
        std::fs::write(temp.path().join(retired.as_str()), b"c").expect("write");
        std::fs::write(temp.path().join(".soma-reflink-probe-1-src"), b"p").expect("write");
        std::fs::create_dir(temp.path().join("head-dir")).expect("mkdir");

        let dir = File::open(temp.path()).expect("open dir");
        let report = reconcile(&ledger, dir.as_fd()).expect("reconcile");
        assert!(!report.is_clean());
        assert!(report.dispositions().contains(&Disposition::Consistent {
            token: token(1),
            name: owned
        }));
        assert!(report.dispositions().contains(&Disposition::Orphan {
            name: orphan,
            retired: false
        }));
        assert!(report.dispositions().contains(&Disposition::Orphan {
            name: retired,
            retired: true
        }));
        assert!(report.dispositions().contains(&Disposition::Missing {
            token: token(2),
            name: missing
        }));
        assert!(report.dispositions().contains(&Disposition::Foreign {
            entry: ".soma-reflink-probe-1-src".to_owned(),
            reason: ForeignReason::InvalidName,
        }));
        assert!(report.dispositions().contains(&Disposition::Foreign {
            entry: "head-dir".to_owned(),
            reason: ForeignReason::NotRegularFile,
        }));
        assert_eq!(report.count(|d| matches!(d, Disposition::Orphan { .. })), 2);
    }

    #[test]
    fn empty_directory_and_empty_ledger_are_clean() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = File::open(temp.path()).expect("open dir");
        let report = reconcile(&HeadLedger::new(), dir.as_fd()).expect("reconcile");
        assert!(report.is_clean());
        assert!(report.dispositions().is_empty());
    }
}
