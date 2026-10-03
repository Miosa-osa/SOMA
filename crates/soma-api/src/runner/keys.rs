use std::{
    collections::HashMap,
    sync::{Arc, RwLock, atomic::AtomicI64},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

pub use crate::runner::principal::{
    Joined, KeyRecord, Principal, Projects, Refusal, Tenant, TenantPolicy,
};
use crate::runner::{feed::FeedEvent, ids::decode_sha256_hex, journal::ExpireReason};

/// The in-memory key and policy table the control-plane feed fills.
///
/// Lookups never touch the network: a key that is not here is refused. The table is replaced
/// whole at the end of a snapshot, so a key revoked while the feed was away disappears with the
/// snapshot that no longer lists it, and deltas between snapshots apply in place.
#[derive(Debug)]
pub struct KeyTable {
    state: RwLock<State>,
    started: Instant,
}

#[derive(Debug, Default)]
struct State {
    live: Entries,
    /// The table being rebuilt between `snapshot_begin` and `snapshot_end`.
    staging: Option<Entries>,
    last_seq: u64,
    last_event: Option<Instant>,
    /// Live sandbox counts per tenant. Kept outside the entries so that neither a snapshot nor
    /// a policy change forgets sandboxes that are running.
    counters: HashMap<String, Arc<AtomicI64>>,
}

/// One record per key hash, already joined with its tenant (contract C3).
#[derive(Debug, Default)]
struct Entries {
    keys: HashMap<[u8; 32], Arc<Joined>>,
    tenants: HashMap<String, Arc<Tenant>>,
}

impl Entries {
    fn upsert_key(&mut self, hash: [u8; 32], key: KeyRecord) {
        let tenant = self.tenants.get(&key.tenant_id).cloned();
        let key = Arc::new(key);
        self.keys.insert(hash, Arc::new(Joined { key, tenant }));
    }

    /// Installs a tenant policy and re-joins that tenant's keys to it.
    ///
    /// This walks the keys, which is the price of a single lookup on every request: a policy
    /// change is rare, an admission is not.
    fn upsert_tenant(&mut self, tenant_id: &str, tenant: &Arc<Tenant>) {
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

/// A feed event the table could not accept, which ends the feed connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedViolation {
    /// `seq` did not increase, so the stream is not the one this table has been following.
    SequenceRegressed { last: u64, received: u64 },
    /// A `key_hash` that is not 64 hex digits.
    MalformedKeyHash,
    /// `snapshot_end` without a `snapshot_begin`.
    UnopenedSnapshot,
}

impl Default for KeyTable {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyTable {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
            started: Instant::now(),
        }
    }

    /// The last sequence number applied, which is the `after` a reconnecting feed asks for.
    #[must_use]
    pub fn last_seq(&self) -> u64 {
        self.read().last_seq
    }

    /// Marks a new feed connection: the last one's half-built snapshot is forgotten.
    pub fn begin_connection(&self) {
        self.write().staging = None;
    }

    /// Counts a skipped malformed line for sequence and age, so a reconnect skips it too.
    pub fn skip(&self, seq: Option<u64>, now: Instant) {
        let mut state = self.write();
        if let Some(seq) = seq.filter(|seq| *seq > state.last_seq) {
            state.last_seq = seq;
        }
        state.last_event = Some(now);
    }

    /// Time since the feed last delivered any event, or since startup if it never has.
    #[must_use]
    pub fn feed_age(&self, now: Instant) -> Duration {
        let last = self.read().last_event.unwrap_or(self.started);
        now.saturating_duration_since(last)
    }

    /// Whether the table has ever been filled by a completed snapshot or a delta.
    #[must_use]
    pub fn has_received(&self) -> bool {
        self.read().last_event.is_some()
    }

    /// Applies one feed event.
    ///
    /// # Errors
    ///
    /// Returns the violation when the event cannot belong to the stream this table follows;
    /// the feed client then reconnects from the last applied sequence number.
    pub fn apply(&self, event: &FeedEvent, now: Instant) -> Result<(), FeedViolation> {
        let mut state = self.write();
        let seq = event.seq();
        // A snapshot is a reset point and may be numbered below the last applied event.
        let resync = matches!(event, FeedEvent::SnapshotBegin { .. });
        if seq <= state.last_seq && !resync {
            return Err(FeedViolation::SequenceRegressed {
                last: state.last_seq,
                received: seq,
            });
        }
        match event {
            FeedEvent::SnapshotBegin { .. } => state.staging = Some(Entries::default()),
            FeedEvent::SnapshotEnd { .. } => {
                let staged = state
                    .staging
                    .take()
                    .ok_or(FeedViolation::UnopenedSnapshot)?;
                state.live = staged;
            }
            FeedEvent::KeyUpsert {
                key_hash,
                key_id,
                tenant_id,
                user_id,
                projects,
                rate_per_s,
                ..
            } => {
                let hash = decode_sha256_hex(key_hash).ok_or(FeedViolation::MalformedKeyHash)?;
                let record = KeyRecord {
                    key_id: key_id.clone(),
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    projects: projects.to_projects(),
                    rate_per_second: *rate_per_s,
                };
                state.target().upsert_key(hash, record);
            }
            FeedEvent::KeyRevoke { key_hash, .. } => {
                let hash = decode_sha256_hex(key_hash).ok_or(FeedViolation::MalformedKeyHash)?;
                // A revoke reaches the live table even mid-snapshot. The snapshot that is
                // being built may have listed the key before the revoke, and the live table
                // must not keep serving it until that snapshot ends.
                state.live.keys.remove(&hash);
                if let Some(staging) = state.staging.as_mut() {
                    staging.keys.remove(&hash);
                }
            }
            FeedEvent::TenantPolicy {
                tenant_id,
                soma,
                suspended,
                max_concurrent_share,
                default_timeout_s,
                max_lifetime_s,
                ..
            } => {
                let policy = TenantPolicy {
                    soma: *soma,
                    suspended: *suspended,
                    max_concurrent_share: *max_concurrent_share,
                    default_timeout_seconds: *default_timeout_s,
                    max_lifetime_seconds: *max_lifetime_s,
                };
                let counter = state.counter(tenant_id);
                let tenant = Arc::new(Tenant::new(policy, counter));
                state.target().upsert_tenant(tenant_id, &tenant);
            }
            FeedEvent::Heartbeat { .. } | FeedEvent::Unknown { .. } => {}
        }
        state.last_seq = seq;
        state.last_event = Some(now);
        Ok(())
    }

    /// Admits the caller behind an `Authorization` header value, or says why not.
    ///
    /// # Errors
    ///
    /// Returns [`Refusal::Unauthorized`] for a missing, non-`msk_`, or unknown token, and
    /// [`Refusal::Forbidden`] for a known key whose tenant may not use the runner.
    pub fn admit(&self, authorization: Option<&str>) -> Result<Principal, Refusal> {
        let token = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|token| token.starts_with("msk_"))
            .ok_or(Refusal::Unauthorized)?;
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let joined = self
            .read()
            .live
            .keys
            .get(&hash)
            .cloned()
            .ok_or(Refusal::Unauthorized)?;
        // A tenant the feed has published no policy for is refused, not assumed enabled: the
        // runner only ever serves tenants the control plane has positively turned on.
        let tenant = joined.tenant.clone().ok_or(Refusal::Forbidden)?;
        if !tenant.policy.soma || tenant.policy.suspended {
            return Err(Refusal::Forbidden);
        }
        Ok(Principal {
            key: Arc::clone(&joined.key),
            tenant,
        })
    }

    /// The live sandbox counter of `tenant_id`, created if the feed has not named it yet.
    #[must_use]
    pub fn counter(&self, tenant_id: &str) -> Arc<AtomicI64> {
        self.write().counter(tenant_id)
    }

    /// Why the feed says `tenant_id` may no longer hold sandboxes on this runner, if it does:
    /// `suspended` or `soma_disabled`, the journal's expire reasons (contract C4).
    ///
    /// Only a published policy says so; a tenant the table has not heard of keeps its
    /// sandboxes, so a feed outage never empties a host.
    #[must_use]
    pub fn reap_reason(&self, tenant_id: &str) -> Option<ExpireReason> {
        let state = self.read();
        let policy = state.live.tenants.get(tenant_id)?.policy;
        if policy.suspended {
            Some(ExpireReason::Suspended)
        } else if !policy.soma {
            Some(ExpireReason::SomaDisabled)
        } else {
            None
        }
    }

    /// The number of keys the live table holds.
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.read().live.keys.len()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl State {
    fn counter(&mut self, tenant_id: &str) -> Arc<AtomicI64> {
        Arc::clone(self.counters.entry(tenant_id.to_owned()).or_default())
    }

    /// Where a data event lands: the snapshot being built, or the live table between snapshots.
    fn target(&mut self) -> &mut Entries {
        self.staging.as_mut().unwrap_or(&mut self.live)
    }
}

#[cfg(test)]
#[path = "keys_sequence_tests.rs"]
mod sequence_tests;
#[cfg(test)]
#[path = "keys_tests.rs"]
pub(crate) mod tests;
