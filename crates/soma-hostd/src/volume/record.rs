//! Fixed-layout, checksummed write-ahead records for the volume lifecycle.
//!
//! Every volume transaction writes a `begin` record before it touches the file system and an
//! `end` record after, so a crash between the two is repaired on start by replaying the records
//! rather than guessed at.
//! The layout is deliberately separate from the worker ledger's [`crate::Record`]: a worker
//! record names a worker and a pool key, and a volume record names a volume, so mixing them in
//! one fixed body would carry a worker's identity on every volume statement.
//! These records must live on durable disk and never on the `tmpfs` registry, which is the rule
//! [`crate::volume::placement`] enforces.

use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::{InstanceId, OperationId, RequestFingerprint, VolumeId};

/// Magic of one volume write-ahead record.
pub const VOLUME_RECORD_MAGIC: &[u8; 8] = b"SOMAVRW\0";
/// The record schema version.
pub const VOLUME_RECORD_VERSION: u16 = 1;

const VOLUME_RECORD_BODY_LEN: usize = 8 + 2 + 1 + 4 + 16 + 16 + 16 + 32 + 8;
/// The exact encoded length of one volume record.
pub const VOLUME_RECORD_LEN: usize = VOLUME_RECORD_BODY_LEN + 32;

/// What one volume record states.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum VolumeRecordKind {
    /// A volume file is about to be created.
    CreateBegin = 1,
    /// The volume file exists and its sidecar names the owner.
    CreateEnd = 2,
    /// The volume is about to be bound to a machine as `vdc`.
    AttachBegin = 3,
    /// The volume is bound and the guest has mounted it.
    AttachEnd = 4,
    /// The guest flush and unmount is about to run.
    DetachBegin = 5,
    /// The file handle is closed and the sidecar says detached.
    DetachEnd = 6,
    /// Destruction is decided; the reaper will finish it.
    DestroyBegin = 7,
    /// The file is unlinked and the data key is destroyed.
    DestroyEnd = 8,
    /// An offline repair ran; `detail` is the fsck outcome code.
    Repair = 9,
    /// An orphan whose owner no longer exists was reclaimed.
    Reaped = 10,
}

impl VolumeRecordKind {
    /// Decodes one kind, or `None` when this build does not know it.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::CreateBegin,
            2 => Self::CreateEnd,
            3 => Self::AttachBegin,
            4 => Self::AttachEnd,
            5 => Self::DetachBegin,
            6 => Self::DetachEnd,
            7 => Self::DestroyBegin,
            8 => Self::DestroyEnd,
            9 => Self::Repair,
            10 => Self::Reaped,
            _ => return None,
        })
    }

    /// Whether this kind opens a transaction that a matching `end` must close.
    #[must_use]
    pub const fn is_begin(self) -> bool {
        matches!(
            self,
            Self::CreateBegin | Self::AttachBegin | Self::DetachBegin | Self::DestroyBegin
        )
    }

    /// Whether this kind closes a transaction.
    #[must_use]
    pub const fn is_end(self) -> bool {
        matches!(
            self,
            Self::CreateEnd | Self::AttachEnd | Self::DetachEnd | Self::DestroyEnd
        )
    }

    /// Whether this kind closes the transaction this begin opened.
    #[must_use]
    pub const fn closes(self, begin: Self) -> bool {
        matches!(
            (begin, self),
            (Self::CreateBegin, Self::CreateEnd)
                | (Self::AttachBegin, Self::AttachEnd)
                | (Self::DetachBegin, Self::DetachEnd)
                | (Self::DestroyBegin, Self::DestroyEnd)
        )
    }
}

/// One durable statement about a volume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeRecord {
    /// The kind.
    pub kind: VolumeRecordKind,
    /// Kind-specific detail, such as an fsck outcome code.
    pub detail: u32,
    /// The volume.
    pub volume: VolumeId,
    /// The owning Instance, when known.
    pub instance: Option<InstanceId>,
    /// The operation, when known.
    pub operation: Option<OperationId>,
    /// The request fingerprint, when known.
    pub fingerprint: Option<RequestFingerprint>,
    /// Unix nanoseconds when the record was written.
    pub time_nanos: u64,
}

impl VolumeRecord {
    /// Starts one record with no optional parts.
    #[must_use]
    pub fn new(kind: VolumeRecordKind, volume: VolumeId) -> Self {
        Self {
            kind,
            detail: 0,
            volume,
            instance: None,
            operation: None,
            fingerprint: None,
            time_nanos: now_nanos(),
        }
    }

    /// Sets the detail.
    #[must_use]
    pub const fn detail(mut self, detail: u32) -> Self {
        self.detail = detail;
        self
    }

    /// Sets the owning Instance.
    #[must_use]
    pub const fn instance(mut self, instance: InstanceId) -> Self {
        self.instance = Some(instance);
        self
    }

    /// Sets the operation.
    #[must_use]
    pub const fn operation(mut self, operation: OperationId) -> Self {
        self.operation = Some(operation);
        self
    }

    /// Sets the fingerprint.
    #[must_use]
    pub const fn fingerprint(mut self, fingerprint: RequestFingerprint) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }

    /// Encodes the record with its trailing checksum.
    #[must_use]
    pub fn encode(&self) -> [u8; VOLUME_RECORD_LEN] {
        let mut out = [0_u8; VOLUME_RECORD_LEN];
        let mut cursor = 0;
        let mut put = |bytes: &[u8]| {
            out[cursor..cursor + bytes.len()].copy_from_slice(bytes);
            cursor += bytes.len();
        };
        put(VOLUME_RECORD_MAGIC);
        put(&VOLUME_RECORD_VERSION.to_be_bytes());
        put(&[self.kind as u8]);
        put(&self.detail.to_be_bytes());
        put(self.volume.as_bytes());
        put(self
            .instance
            .map_or([0; 16], |id| *id.as_bytes())
            .as_slice());
        put(self
            .operation
            .map_or([0; 16], |id| *id.as_bytes())
            .as_slice());
        put(self
            .fingerprint
            .map_or([0; 32], |id| *id.as_bytes())
            .as_slice());
        put(&self.time_nanos.to_be_bytes());
        let checksum: [u8; 32] = Sha256::digest(&out[..VOLUME_RECORD_BODY_LEN]).into();
        out[VOLUME_RECORD_BODY_LEN..].copy_from_slice(&checksum);
        out
    }

    /// Decodes one exact record; any malformed, short, long, or mismatched input is `None`.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != VOLUME_RECORD_LEN || &bytes[..8] != VOLUME_RECORD_MAGIC {
            return None;
        }
        let checksum: [u8; 32] = Sha256::digest(&bytes[..VOLUME_RECORD_BODY_LEN]).into();
        if checksum != bytes[VOLUME_RECORD_BODY_LEN..] {
            return None;
        }
        let mut cursor = 8;
        let mut take = |n: usize| {
            let slice = &bytes[cursor..cursor + n];
            cursor += n;
            slice
        };
        if u16::from_be_bytes(array(take(2))) != VOLUME_RECORD_VERSION {
            return None;
        }
        let kind = VolumeRecordKind::from_code(take(1)[0])?;
        let detail = u32::from_be_bytes(array(take(4)));
        let volume = VolumeId::new(array(take(16))).ok()?;
        let instance = InstanceId::new(array(take(16))).ok();
        let operation = OperationId::new(array(take(16))).ok();
        let fingerprint = RequestFingerprint::new(array(take(32))).ok();
        let time_nanos = u64::from_be_bytes(array(take(8)));
        Some(Self {
            kind,
            detail,
            volume,
            instance,
            operation,
            fingerprint,
            time_nanos,
        })
    }
}

fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(slice);
    out
}

/// Unix nanoseconds now, saturated; zero only when the clock precedes the epoch.
#[must_use]
pub fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}
#[cfg(test)]
mod tests;
