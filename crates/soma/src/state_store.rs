use std::{error::Error, fmt};

use crate::InstanceId;

mod memory;

pub use memory::MemoryStateStore;

pub const MAX_STATE_RECORD_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateStoreFailureKind {
    InvalidRecord,
    Conflict,
    Unavailable,
    Corrupt,
    UnsupportedVersion,
    CapacityExceeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateStoreFailure {
    kind: StateStoreFailureKind,
}

impl StateStoreFailure {
    #[must_use]
    pub const fn new(kind: StateStoreFailureKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> StateStoreFailureKind {
        self.kind
    }
}

impl fmt::Display for StateStoreFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("durable machine state operation failed")
    }
}

impl Error for StateStoreFailure {}

#[derive(Clone, PartialEq, Eq)]
pub struct StateRecord(Vec<u8>);

impl StateRecord {
    /// Creates a bounded opaque state document.
    ///
    /// # Errors
    ///
    /// Returns an invalid-record failure when the document is empty or exceeds
    /// [`MAX_STATE_RECORD_BYTES`]. Its internal schema is validated separately by the facade.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, StateStoreFailure> {
        if bytes.is_empty() || bytes.len() > MAX_STATE_RECORD_BYTES {
            return Err(StateStoreFailure::new(StateStoreFailureKind::InvalidRecord));
        }
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for StateRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StateRecord")
            .field("bytes", &self.0.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateRevision(u64);

impl StateRevision {
    pub const INITIAL: Self = Self(1);

    /// Creates a nonzero store revision.
    ///
    /// # Errors
    ///
    /// Returns an invalid-record failure for zero, which is never a committed revision.
    pub const fn new(value: u64) -> Result<Self, StateStoreFailure> {
        if value == 0 {
            return Err(StateStoreFailure::new(StateStoreFailureKind::InvalidRecord));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn next(self) -> Result<Self, StateStoreFailure> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or_else(|| StateStoreFailure::new(StateStoreFailureKind::CapacityExceeded))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredState {
    revision: StateRevision,
    record: StateRecord,
}

impl StoredState {
    #[must_use]
    pub const fn new(revision: StateRevision, record: StateRecord) -> Self {
        Self { revision, record }
    }

    #[must_use]
    pub const fn revision(&self) -> StateRevision {
        self.revision
    }

    #[must_use]
    pub const fn record(&self) -> &StateRecord {
        &self.record
    }
}

/// Atomically stores bounded, facade-owned lifecycle documents by exact Instance ID.
///
/// Implementations must make `create` exclusive and `compare_exchange` atomic across all
/// processes sharing the store. The opaque record must be returned byte-for-byte.
pub trait StateStore: Send {
    /// Creates the first revision when the Instance ID is absent.
    ///
    /// # Errors
    ///
    /// Returns `Conflict` when the key exists, or another typed storage failure.
    fn create(
        &mut self,
        instance_id: &InstanceId,
        record: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure>;

    /// Loads the current document and revision without inventing missing state.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the store cannot safely read the key.
    fn load(&mut self, instance_id: &InstanceId) -> Result<Option<StoredState>, StateStoreFailure>;

    /// Replaces a document only when the durable revision equals `expected`.
    ///
    /// # Errors
    ///
    /// Returns `Conflict` for a stale revision, or another typed storage failure.
    fn compare_exchange(
        &mut self,
        instance_id: &InstanceId,
        expected: StateRevision,
        replacement: StateRecord,
    ) -> Result<StateRevision, StateStoreFailure>;

    /// Reports every Instance ID this store currently holds a record for.
    ///
    /// The identities are returned rather than the documents, because a caller enumerating the
    /// store still has to read each record under its own lock to see a consistent one, and
    /// returning documents here would hand back a set assembled from many different moments.
    ///
    /// This is not a snapshot. A record created or released while the enumeration is running may
    /// or may not appear, so a caller that must not report a released Instance reads each one
    /// back before it does.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the store cannot safely enumerate its records. A store that
    /// found a name it cannot account for reports `Corrupt` rather than omitting it, because an
    /// omission would be indistinguishable from an Instance that does not exist.
    fn list(&mut self) -> Result<Vec<InstanceId>, StateStoreFailure>;
}
