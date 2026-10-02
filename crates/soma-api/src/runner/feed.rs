use serde::Deserialize;

use crate::runner::keys::Projects;

/// One line of the control-plane key and policy feed (contract C3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeedEvent {
    SnapshotBegin {
        seq: u64,
    },
    KeyUpsert {
        seq: u64,
        key_hash: String,
        key_id: String,
        tenant_id: String,
        user_id: Option<String>,
        projects: ProjectScope,
        rate_per_s: Option<u32>,
    },
    KeyRevoke {
        seq: u64,
        key_hash: String,
    },
    TenantPolicy {
        seq: u64,
        tenant_id: String,
        soma: bool,
        suspended: bool,
        max_concurrent: Option<u32>,
    },
    SnapshotEnd {
        seq: u64,
    },
    Heartbeat {
        seq: u64,
    },
    /// An event kind this runner does not know yet. It still counts for sequence and age, so a
    /// newer control plane can add kinds without breaking older runners.
    Unknown {
        seq: u64,
    },
}

/// `projects` on the wire: the string `"*"` or a list of project ids.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ProjectScope {
    Wildcard(Wildcard),
    Only(Vec<String>),
}

/// The literal `"*"`, and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Wildcard {
    #[serde(rename = "*")]
    All,
}

impl ProjectScope {
    #[must_use]
    pub fn to_projects(&self) -> Projects {
        match self {
            Self::Wildcard(Wildcard::All) => Projects::All,
            Self::Only(projects) => Projects::Only(projects.iter().cloned().collect()),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Known {
    SnapshotBegin {
        seq: u64,
    },
    KeyUpsert {
        seq: u64,
        key_hash: String,
        key_id: String,
        tenant_id: String,
        user_id: Option<String>,
        projects: ProjectScope,
        rate_per_s: Option<u32>,
    },
    KeyRevoke {
        seq: u64,
        key_hash: String,
    },
    TenantPolicy {
        seq: u64,
        tenant_id: String,
        soma: bool,
        suspended: bool,
        max_concurrent: Option<u32>,
    },
    SnapshotEnd {
        seq: u64,
    },
    Heartbeat {
        seq: u64,
    },
}

#[derive(Deserialize)]
struct Header {
    seq: u64,
    kind: String,
}

const KNOWN_KINDS: [&str; 6] = [
    "snapshot_begin",
    "key_upsert",
    "key_revoke",
    "tenant_policy",
    "snapshot_end",
    "heartbeat",
];

impl FeedEvent {
    /// Parses one feed line.
    ///
    /// # Errors
    ///
    /// Returns the JSON error for a line that is not an event, or a known event that is missing
    /// a field it requires. An unknown kind is not an error.
    pub fn parse(line: &str) -> Result<Self, serde_json::Error> {
        match serde_json::from_str::<Known>(line) {
            Ok(known) => Ok(known.into()),
            Err(error) => {
                let header: Header = serde_json::from_str(line)?;
                if KNOWN_KINDS.contains(&header.kind.as_str()) {
                    Err(error)
                } else {
                    Ok(Self::Unknown { seq: header.seq })
                }
            }
        }
    }

    #[must_use]
    pub const fn seq(&self) -> u64 {
        match self {
            Self::SnapshotBegin { seq }
            | Self::KeyUpsert { seq, .. }
            | Self::KeyRevoke { seq, .. }
            | Self::TenantPolicy { seq, .. }
            | Self::SnapshotEnd { seq }
            | Self::Heartbeat { seq }
            | Self::Unknown { seq } => *seq,
        }
    }
}

impl From<Known> for FeedEvent {
    fn from(known: Known) -> Self {
        match known {
            Known::SnapshotBegin { seq } => Self::SnapshotBegin { seq },
            Known::KeyUpsert {
                seq,
                key_hash,
                key_id,
                tenant_id,
                user_id,
                projects,
                rate_per_s,
            } => Self::KeyUpsert {
                seq,
                key_hash,
                key_id,
                tenant_id,
                user_id,
                projects,
                rate_per_s,
            },
            Known::KeyRevoke { seq, key_hash } => Self::KeyRevoke { seq, key_hash },
            Known::TenantPolicy {
                seq,
                tenant_id,
                soma,
                suspended,
                max_concurrent,
            } => Self::TenantPolicy {
                seq,
                tenant_id,
                soma,
                suspended,
                max_concurrent,
            },
            Known::SnapshotEnd { seq } => Self::SnapshotEnd { seq },
            Known::Heartbeat { seq } => Self::Heartbeat { seq },
        }
    }
}

#[cfg(test)]
#[path = "feed_tests.rs"]
mod tests;
