//! The key table's records: one per key hash, joined with its tenant.

use std::{collections::HashMap, sync::Arc};

use crate::runner::principal::{Joined, KeyRecord, Tenant};

/// One record per key hash, already joined with its tenant (contract C3).
#[derive(Debug, Default)]
pub(super) struct Entries {
    pub(super) keys: HashMap<[u8; 32], Arc<Joined>>,
    pub(super) tenants: HashMap<String, Arc<Tenant>>,
}

impl Entries {
    pub(super) fn upsert_key(&mut self, hash: [u8; 32], key: KeyRecord) {
        let tenant = self.tenants.get(&key.tenant_id).cloned();
        let key = Arc::new(key);
        self.keys.insert(hash, Arc::new(Joined { key, tenant }));
    }

    /// Installs a tenant policy and re-joins that tenant's keys to it.
    ///
    /// This walks the keys, which is the price of a single lookup on every request: a policy
    /// change is rare, an admission is not.
    pub(super) fn upsert_tenant(&mut self, tenant_id: &str, tenant: &Arc<Tenant>) {
        self.tenants
            .insert(tenant_id.to_owned(), Arc::clone(tenant));
        for joined in self.keys.values_mut() {
            if joined.key.tenant_id == tenant_id {
                *joined = Arc::new(Joined {
                    key: Arc::clone(&joined.key),
                    tenant: Some(Arc::clone(tenant)),
                });
            }
        }
    }
}
