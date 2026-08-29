//! Fixed-layout ledger records.

use crate::{
    BundleId, CleanupGeneration, ConntrackZone, Error, InstanceId, IntentDigest, NetworkIntent,
    OperationId, ProfileDigest, intent::MAX_ENCODED_INTENT,
};

const MAGIC: &[u8; 8] = b"SOMANETL";
const VERSION: u16 = 1;
const HEADER: usize = 8 + 2 + 16 + 4 + 16 + 16 + 32 + 32 + 6 + 4 + 4 + 2 + 4 + 8 + 2;

/// The largest encoded assignment record.
pub const MAX_RECORD: usize = HEADER + MAX_ENCODED_INTENT;

/// One durable assignment: everything the kernel state must match later.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssignmentRecord {
    /// The bundle.
    pub bundle: BundleId,
    /// The cleanup generation.
    pub generation: CleanupGeneration,
    /// The owning Instance.
    pub instance: InstanceId,
    /// The assigning operation.
    pub operation: OperationId,
    /// The admitted profile digest.
    pub profile: ProfileDigest,
    /// The admitted intent digest.
    pub intent_digest: IntentDigest,
    /// The guest MAC.
    pub guest_mac: [u8; 6],
    /// The guest lease index.
    pub lease_index: u32,
    /// The transit lease index.
    pub transit_index: u32,
    /// The conntrack zone.
    pub zone: ConntrackZone,
    /// The vsock CID delivered in the launch page.
    pub vsock_cid: u32,
    /// The wall-clock sample delivered in the launch page, in Unix nanoseconds.
    pub time_sample_nanos: u64,
    /// The admitted intent.
    pub intent: NetworkIntent,
}

impl AssignmentRecord {
    /// Returns whether a replay carries the same operation, Instance, and intent.
    #[must_use]
    pub fn replays(&self, other: &Self) -> bool {
        self.operation == other.operation
            && self.instance == other.instance
            && self.intent_digest == other.intent_digest
            && self.vsock_cid == other.vsock_cid
    }

    /// Encodes the record.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let intent = self.intent.encode();
        let mut out = Vec::with_capacity(HEADER + intent.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(self.bundle.as_bytes());
        out.extend_from_slice(&self.generation.get().to_be_bytes());
        out.extend_from_slice(self.instance.as_bytes());
        out.extend_from_slice(self.operation.as_bytes());
        out.extend_from_slice(&self.profile.0);
        out.extend_from_slice(&self.intent_digest.0);
        out.extend_from_slice(&self.guest_mac);
        out.extend_from_slice(&self.lease_index.to_be_bytes());
        out.extend_from_slice(&self.transit_index.to_be_bytes());
        out.extend_from_slice(&self.zone.get().to_be_bytes());
        out.extend_from_slice(&self.vsock_cid.to_be_bytes());
        out.extend_from_slice(&self.time_sample_nanos.to_be_bytes());
        out.extend_from_slice(
            &u16::try_from(intent.len())
                .unwrap_or(u16::MAX)
                .to_be_bytes(),
        );
        out.extend_from_slice(&intent);
        out
    }

    /// Decodes one record, verifying the digest against the embedded intent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LedgerCorrupt`] for any malformed, short, long, or inconsistent input.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER || bytes.len() > MAX_RECORD || &bytes[..8] != MAGIC {
            return Err(Error::LedgerCorrupt);
        }
        let mut cursor = 8;
        let mut take = |n: usize| {
            let slice = &bytes[cursor..cursor + n];
            cursor += n;
            slice
        };
        if u16::from_be_bytes(array(take(2))) != VERSION {
            return Err(Error::LedgerCorrupt);
        }
        let bundle = BundleId::new(array(take(16))).map_err(|_| Error::LedgerCorrupt)?;
        let generation = CleanupGeneration::new(u32::from_be_bytes(array(take(4))))
            .map_err(|_| Error::LedgerCorrupt)?;
        let instance = InstanceId::new(array(take(16))).map_err(|_| Error::LedgerCorrupt)?;
        let operation = OperationId::new(array(take(16))).map_err(|_| Error::LedgerCorrupt)?;
        let profile = ProfileDigest(array(take(32)));
        let intent_digest = IntentDigest(array(take(32)));
        let guest_mac = array(take(6));
        let lease_index = u32::from_be_bytes(array(take(4)));
        let transit_index = u32::from_be_bytes(array(take(4)));
        let zone = ConntrackZone::new(u16::from_be_bytes(array(take(2))))
            .map_err(|_| Error::LedgerCorrupt)?;
        let vsock_cid = u32::from_be_bytes(array(take(4)));
        let time_sample_nanos = u64::from_be_bytes(array(take(8)));
        let intent_len = usize::from(u16::from_be_bytes(array(take(2))));
        if bytes.len() != HEADER + intent_len {
            return Err(Error::LedgerCorrupt);
        }
        let intent = NetworkIntent::decode(&bytes[HEADER..]).map_err(|_| Error::LedgerCorrupt)?;
        if intent.digest() != intent_digest || intent.profile() != profile {
            return Err(Error::LedgerCorrupt);
        }
        Ok(Self {
            bundle,
            generation,
            instance,
            operation,
            profile,
            intent_digest,
            guest_mac,
            lease_index,
            transit_index,
            zone,
            vsock_cid,
            time_sample_nanos,
            intent,
        })
    }
}

fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(slice);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::EgressClass;

    pub(crate) fn record(operation: u8, vsock_cid: u32) -> AssignmentRecord {
        let intent = NetworkIntent::new(
            EgressClass::PublicInternet,
            vec![std::net::Ipv4Addr::new(1, 1, 1, 1)],
            Vec::new(),
            ProfileDigest([3; 32]),
        )
        .expect("intent");
        AssignmentRecord {
            bundle: BundleId::new([1; 16]).expect("id"),
            generation: CleanupGeneration::new(1).expect("generation"),
            instance: InstanceId::new([2; 16]).expect("id"),
            operation: OperationId::new([operation; 16]).expect("id"),
            profile: ProfileDigest([3; 32]),
            intent_digest: intent.digest(),
            guest_mac: [2, 1, 2, 3, 4, 5],
            lease_index: 7,
            transit_index: 7,
            zone: ConntrackZone::new(8).expect("zone"),
            vsock_cid,
            time_sample_nanos: 1,
            intent,
        }
    }

    #[test]
    fn record_round_trips_and_rejects_corruption() {
        let sample = record(4, 33);
        let encoded = sample.encode();
        assert_eq!(AssignmentRecord::decode(&encoded).expect("decodes"), sample);
        for index in 0..encoded.len() {
            let mut flipped = encoded.clone();
            flipped[index] ^= 0x80;
            let decoded = AssignmentRecord::decode(&flipped);
            if index >= HEADER - 2 {
                assert_eq!(decoded, Err(Error::LedgerCorrupt), "byte {index}");
            } else {
                assert!(decoded.is_err() || decoded.as_ref().is_ok_and(|r| r != &sample));
            }
        }
        assert_eq!(
            AssignmentRecord::decode(&encoded[..encoded.len() - 1]),
            Err(Error::LedgerCorrupt)
        );
        let mut long = encoded.clone();
        long.push(0);
        assert_eq!(AssignmentRecord::decode(&long), Err(Error::LedgerCorrupt));
        assert!(sample.replays(&record(4, 33)));
        assert!(!sample.replays(&record(5, 33)));
        assert!(!sample.replays(&record(4, 34)));
    }
}
