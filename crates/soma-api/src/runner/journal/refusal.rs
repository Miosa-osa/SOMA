//! Why a create was refused before it launched.

use serde::{Deserialize, Serialize};

/// Why a create was refused before it launched.
///
/// Both refusals answer `429`, so the status alone cannot tell an operator which one they had: a
/// runner that is at its own in-flight ceiling is a different problem from a tenant that is at its
/// share, and only the second is about the tenant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    /// This runner already has as many creates in flight as it admits (`429 runtime_busy`).
    RuntimeBusy,
    /// The tenant's share on this runner is full (contract C3, `429 rate_limited`).
    TenantShare,
}
