//! A deterministic MCP result for enumerating managed sandboxes.

use serde::Serialize;
use soma::SandboxEntry;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListResult {
    entries: Vec<SandboxEntry>,
}

impl ListResult {
    #[must_use]
    pub const fn new(entries: Vec<SandboxEntry>) -> Self {
        Self { entries }
    }

    #[must_use]
    pub fn body(&self) -> ListBody<'_> {
        ListBody {
            count: self.entries.len(),
            sandboxes: self.entries.iter().map(EntryBody::from).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct ListBody<'a> {
    count: usize,
    sandboxes: Vec<EntryBody<'a>>,
}

#[derive(Serialize)]
struct EntryBody<'a> {
    instance_id: &'a str,
    phase: &'static str,
    backend: soma::BackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    liveness: &'static str,
}

impl<'a> From<&'a SandboxEntry> for EntryBody<'a> {
    fn from(entry: &'a SandboxEntry) -> Self {
        Self {
            instance_id: entry.instance_id().as_str(),
            phase: entry.phase().code(),
            backend: entry.backend(),
            name: entry.name().map(soma::MachineName::as_str),
            liveness: entry.liveness().code(),
        }
    }
}
