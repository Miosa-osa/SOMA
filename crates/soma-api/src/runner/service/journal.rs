//! What one request writes to the paperwork journal.
//!
//! Three shapes: the entry for a request this runner served, the entry for an id it could not
//! address here, and the one place a response takes its journal entry. They live apart from the
//! request path so that the rules about what is billable are readable on their own.

use crate::runner::{
    ids::SandboxId,
    journal::{Entry, EntryKind},
    keys::Principal,
};

use super::RunnerResponse;

pub(super) fn entry(
    kind: EntryKind,
    principal: &Principal,
    project_id: Option<String>,
    sandbox: Option<&SandboxId>,
    status: u16,
) -> Entry {
    Entry {
        kind,
        tenant_id: principal.key.tenant_id.clone(),
        key_id: Some(principal.key.key_id.clone()),
        project_id,
        sandbox_id: sandbox.map(ToString::to_string),
        status,
        ms: 0,
        exit_code: None,
        cpu_ms: None,
        lifetime_ms: None,
        reason: None,
    }
}

/// Journals an id the caller could not address here: malformed ids are paperwork, a `421` is
/// not, because the runner that owns the sandbox journals the request it actually serves.
pub(super) fn refused_sandbox(
    response: RunnerResponse,
    kind: EntryKind,
    principal: &Principal,
) -> RunnerResponse {
    if response.status == 421 {
        return response;
    }
    let status = response.status;
    with_journal(response, Some(entry(kind, principal, None, None, status)))
}

pub(super) fn with_journal(mut response: RunnerResponse, entry: Option<Entry>) -> RunnerResponse {
    response.journal = entry;
    response
}
