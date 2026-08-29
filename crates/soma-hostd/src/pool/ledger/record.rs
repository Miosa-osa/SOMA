//! Fixed-layout, checksummed ledger records.

use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::{
    InstanceId, LeaseGeneration, OperationId, PoolKeyDigest, RequestFingerprint, ResourceRefs,
    WorkerId, WorkerIdentity,
};

const MAGIC: &[u8; 8] = b"SOMAHOST";
const VERSION: u16 = 1;
const BODY_LEN: usize =
    8 + 2 + 1 + 1 + 16 + 8 + 32 + 16 + 16 + 32 + ResourceRefs::LEN + WorkerIdentity::LEN + 8;

/// The exact encoded length of one record.
pub const RECORD_LEN: usize = BODY_LEN + 32;

/// What one record states about a worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RecordKind {
    /// A slot was opened for a new worker.
    Constructing = 1,
    /// Construction finished; the worker holds only invariant state.
    Sterile = 2,
    /// Construction failed; the worker is dead.
    ConstructFailed = 3,
    /// One claim won; `detail` is the claim class.
    Claiming = 4,
    /// One transfer step was acknowledged; `detail` is the step.
    TransferStep = 5,
    /// One transfer step failed; `detail` is the step.
    TransferFault = 6,
    /// Every step was acknowledged; the worker belongs to the Instance.
    Assigned = 7,
    /// The Instance started.
    Running = 8,
    /// Teardown began; `detail` is the reason.
    Destroying = 9,
    /// Teardown finished.
    Dead = 10,
    /// A restart found the entry nonterminal.
    Suspect = 11,
    /// Reconciliation decided; `detail` is the disposition.
    Reconciled = 12,
}

impl RecordKind {
    /// Decodes one kind.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Constructing,
            2 => Self::Sterile,
            3 => Self::ConstructFailed,
            4 => Self::Claiming,
            5 => Self::TransferStep,
            6 => Self::TransferFault,
            7 => Self::Assigned,
            8 => Self::Running,
            9 => Self::Destroying,
            10 => Self::Dead,
            11 => Self::Suspect,
            12 => Self::Reconciled,
            _ => return None,
        })
    }
}

/// One durable statement about a worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Record {
    /// The kind.
    pub kind: RecordKind,
    /// Kind-specific detail.
    pub detail: u8,
    /// The worker.
    pub worker: WorkerId,
    /// The lease generation.
    pub lease_generation: LeaseGeneration,
    /// The pool key digest.
    pub key: PoolKeyDigest,
    /// The operation, when known.
    pub operation: Option<OperationId>,
    /// The Instance, when known.
    pub instance: Option<InstanceId>,
    /// The claim fingerprint, when known.
    pub fingerprint: Option<RequestFingerprint>,
    /// Resource references.
    pub resources: ResourceRefs,
    /// The process identity, when known.
    pub identity: Option<WorkerIdentity>,
    /// Unix nanoseconds when the record was written.
    pub time_nanos: u64,
}

impl Record {
    /// Starts one record with no optional parts.
    #[must_use]
    pub fn new(
        kind: RecordKind,
        worker: WorkerId,
        lease_generation: LeaseGeneration,
        key: PoolKeyDigest,
    ) -> Self {
        Self {
            kind,
            detail: 0,
            worker,
            lease_generation,
            key,
            operation: None,
            instance: None,
            fingerprint: None,
            resources: ResourceRefs::default(),
            identity: None,
            time_nanos: now_nanos(),
        }
    }

    /// Sets the detail byte.
    #[must_use]
    pub const fn detail(mut self, detail: u8) -> Self {
        self.detail = detail;
        self
    }

    /// Sets the operation.
    #[must_use]
    pub const fn operation(mut self, operation: OperationId) -> Self {
        self.operation = Some(operation);
        self
    }

    /// Sets the Instance.
    #[must_use]
    pub const fn instance(mut self, instance: InstanceId) -> Self {
        self.instance = Some(instance);
        self
    }

    /// Sets the fingerprint.
    #[must_use]
    pub const fn fingerprint(mut self, fingerprint: RequestFingerprint) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }

    /// Sets the resource references.
    #[must_use]
    pub const fn resources(mut self, resources: ResourceRefs) -> Self {
        self.resources = resources;
        self
    }

    /// Sets the process identity.
    #[must_use]
    pub const fn identity(mut self, identity: WorkerIdentity) -> Self {
        self.identity = Some(identity);
        self
    }

    /// Encodes the record with its trailing checksum.
    #[must_use]
    pub fn encode(&self) -> [u8; RECORD_LEN] {
        let mut out = [0; RECORD_LEN];
        let mut cursor = 0;
        let mut put = |bytes: &[u8]| {
            out[cursor..cursor + bytes.len()].copy_from_slice(bytes);
            cursor += bytes.len();
        };
        put(MAGIC);
        put(&VERSION.to_be_bytes());
        put(&[self.kind as u8, self.detail]);
        put(self.worker.as_bytes());
        put(&self.lease_generation.get().to_be_bytes());
        put(self.key.as_bytes());
        put(self
            .operation
            .map_or([0; 16], |id| *id.as_bytes())
            .as_slice());
        put(self
            .instance
            .map_or([0; 16], |id| *id.as_bytes())
            .as_slice());
        put(self
            .fingerprint
            .map_or([0; 32], |id| *id.as_bytes())
            .as_slice());
        put(&self.resources.encode());
        put(&self
            .identity
            .map_or([0; WorkerIdentity::LEN], |id| id.encode()));
        put(&self.time_nanos.to_be_bytes());
        let checksum: [u8; 32] = Sha256::digest(&out[..BODY_LEN]).into();
        out[BODY_LEN..].copy_from_slice(&checksum);
        out
    }

    /// Decodes one exact record; any malformed, short, long, or mismatched input is `None`.
    #[must_use]
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != RECORD_LEN || &bytes[..8] != MAGIC {
            return None;
        }
        let checksum: [u8; 32] = Sha256::digest(&bytes[..BODY_LEN]).into();
        if checksum != bytes[BODY_LEN..] {
            return None;
        }
        let mut cursor = 8;
        let mut take = |n: usize| {
            let slice = &bytes[cursor..cursor + n];
            cursor += n;
            slice
        };
        if u16::from_be_bytes(array(take(2))) != VERSION {
            return None;
        }
        let kind = RecordKind::from_code(take(1)[0])?;
        let detail = take(1)[0];
        let worker = WorkerId::new(array(take(16))).ok()?;
        let lease_generation = LeaseGeneration::new(u64::from_be_bytes(array(take(8)))).ok()?;
        let key = PoolKeyDigest::from_bytes(array(take(32)));
        let operation = OperationId::new(array(take(16))).ok();
        let instance = InstanceId::new(array(take(16))).ok();
        let fingerprint = RequestFingerprint::new(array(take(32))).ok();
        let resources = ResourceRefs::decode(&array(take(ResourceRefs::LEN)));
        let identity = WorkerIdentity::decode(&array(take(WorkerIdentity::LEN)));
        let time_nanos = u64::from_be_bytes(array(take(8)));
        Some(Self {
            kind,
            detail,
            worker,
            lease_generation,
            key,
            operation,
            instance,
            fingerprint,
            resources,
            identity,
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
pub(crate) mod tests {
    use super::*;

    pub(crate) fn record(kind: RecordKind, worker: u8) -> Record {
        Record::new(
            kind,
            WorkerId::new([worker; 16]).expect("worker"),
            LeaseGeneration::FIRST,
            PoolKeyDigest::from_bytes([5; 32]),
        )
        .operation(OperationId::new([2; 16]).expect("operation"))
        .fingerprint(RequestFingerprint::of(b"x"))
        .identity(WorkerIdentity {
            process: 7,
            token: [8; 16],
        })
    }

    #[test]
    fn records_round_trip_and_every_bit_flip_is_rejected() {
        let sample = record(RecordKind::Claiming, 1).detail(3);
        let encoded = sample.encode();
        assert_eq!(Record::decode(&encoded), Some(sample));
        for index in 0..RECORD_LEN {
            let mut flipped = encoded;
            flipped[index] ^= 0x01;
            assert_eq!(Record::decode(&flipped), None, "byte {index}");
        }
        assert_eq!(Record::decode(&encoded[..RECORD_LEN - 1]), None);
        let mut long = encoded.to_vec();
        long.push(0);
        assert_eq!(Record::decode(&long), None);
        assert_eq!(RecordKind::from_code(0), None);
        assert!(now_nanos() > 0);
    }
}
