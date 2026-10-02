use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use crate::runner::feed::FeedEvent;
pub use crate::runner::principal::{KeyRecord, Principal, Projects, Refusal, TenantPolicy};

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
    /// Set when a feed connection opens and cleared by its first event.
    connection_opened: bool,
}

#[derive(Debug, Default)]
struct Entries {
    keys: HashMap<[u8; 32], Arc<KeyRecord>>,
    tenants: HashMap<String, TenantPolicy>,
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

    /// Marks the start of a feed connection.
    ///
    /// Any half-built snapshot of the previous connection is forgotten; the live table stays as
    /// it was. A snapshot that opens the new connection may restart the sequence below the last
    /// applied number, which is how a control plane that lost its own sequence (a restart) brings
    /// a runner back without the runner refusing it forever.
    pub fn begin_connection(&self) {
        let mut state = self.write();
        state.staging = None;
        state.connection_opened = true;
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
        let resync = state.connection_opened && matches!(event, FeedEvent::SnapshotBegin { .. });
        state.connection_opened = false;
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
                let hash = decode_hash(key_hash).ok_or(FeedViolation::MalformedKeyHash)?;
                let record = Arc::new(KeyRecord {
                    key_id: key_id.clone(),
                    tenant_id: tenant_id.clone(),
                    user_id: user_id.clone(),
                    projects: projects.to_projects(),
                    rate_per_second: *rate_per_s,
                });
                state.target().keys.insert(hash, record);
            }
            FeedEvent::KeyRevoke { key_hash, .. } => {
                let hash = decode_hash(key_hash).ok_or(FeedViolation::MalformedKeyHash)?;
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
                max_concurrent,
                ..
            } => {
                let policy = TenantPolicy {
                    soma: *soma,
                    suspended: *suspended,
                    max_concurrent: *max_concurrent,
                };
                state.target().tenants.insert(tenant_id.clone(), policy);
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
        let state = self.read();
        let key = state
            .live
            .keys
            .get(&hash)
            .cloned()
            .ok_or(Refusal::Unauthorized)?;
        // A tenant the feed has published no policy for is refused, not assumed enabled: the
        // runner only ever serves tenants the control plane has positively turned on.
        let policy = state
            .live
            .tenants
            .get(&key.tenant_id)
            .copied()
            .ok_or(Refusal::Forbidden)?;
        if !policy.soma || policy.suspended {
            return Err(Refusal::Forbidden);
        }
        Ok(Principal { key, policy })
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
    /// Where a data event lands: the snapshot being built, or the live table between snapshots.
    fn target(&mut self) -> &mut Entries {
        self.staging.as_mut().unwrap_or(&mut self.live)
    }
}

/// Decodes the lowercase hex SHA-256 the control plane publishes as `key_hash`.
fn decode_hash(hex: &str) -> Option<[u8; 32]> {
    let bytes = hex.as_bytes();
    if bytes.len() != 64 {
        return None;
    }
    let mut hash = [0_u8; 32];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (slot, pair) in hash.iter_mut().zip(pairs) {
        *slot = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Some(hash)
}

const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
pub(crate) mod tests;
