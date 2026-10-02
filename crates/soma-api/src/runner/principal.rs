use std::{collections::HashSet, sync::Arc};

/// One API key the control plane has published to this runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRecord {
    pub key_id: String,
    pub tenant_id: String,
    pub user_id: Option<String>,
    pub projects: Projects,
    pub rate_per_second: Option<u32>,
}

/// The projects a key may create sandboxes in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Projects {
    All,
    Only(HashSet<String>),
}

impl Projects {
    /// Whether a create naming `project` is inside this key's scope.
    ///
    /// A create that names no project is allowed, as it is on the standard path, where
    /// `authorize_project/2` passes a request without a `project_id`.
    #[must_use]
    pub fn allows(&self, project: Option<&str>) -> bool {
        match (self, project) {
            (Self::All, _) | (_, None) => true,
            (Self::Only(projects), Some(project)) => projects.contains(project),
        }
    }
}

/// One tenant's runner policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TenantPolicy {
    pub soma: bool,
    pub suspended: bool,
    pub max_concurrent: Option<u32>,
}

/// Why a caller was not admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No `Bearer msk_` token, or a token the table does not hold (401).
    Unauthorized,
    /// The key is known but its tenant may not use the runner (403).
    Forbidden,
}

/// The identity a request acts under once its key is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub key: Arc<KeyRecord>,
    pub policy: TenantPolicy,
}
