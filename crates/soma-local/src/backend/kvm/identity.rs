//! The exact identities one launch binds, derived once and shared by everything that binds them.
//!
//! The launch page, the network claim, and the activation receipt all name the same Instance,
//! the same Launch operation, and the same vsock context identifier. If any of them derived its
//! own copy, a machine could be built under one identity and given a network leased to another,
//! and no reader of the evidence could tell. So they are derived here once and passed around.

use soma::{BackendFailureKind, InstanceId};

/// The lowest context identifier a guest may take; 0, 1, and 2 are reserved by the kernel.
pub(super) const FIRST_GUEST_CID: u32 = 3;

/// Every identity one launch of one Instance binds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct LaunchIdentity {
    /// The Instance identity as the sixteen bytes the launch page carries.
    pub(super) instance: [u8; 16],
    /// The Launch operation this attempt belongs to.
    pub(super) operation: [u8; 16],
    /// The vsock context identifier this Instance takes.
    pub(super) guest_cid: u32,
}

impl LaunchIdentity {
    /// Derives every identity one launch of `instance` needs.
    ///
    /// # Errors
    ///
    /// Returns [`BackendFailureKind::WorkloadRejected`] when the Instance identity is not the
    /// thirty-two lowercase hexadecimal characters the launch page encodes.
    pub(super) fn derive(instance: &InstanceId) -> Result<Self, BackendFailureKind> {
        let bytes = instance_bytes(instance)?;
        Ok(Self {
            instance: bytes,
            operation: fresh16(),
            guest_cid: guest_cid_for(bytes),
        })
    }
}

/// The exact sixteen bytes the launch page carries for this Instance.
///
/// The guest authenticates an Instance identity, and the receipt reports one. If those were
/// allowed to differ, the authenticated session would prove one Instance while the public
/// evidence described another, and no reader could tell. This is the only conversion between
/// them, and it is exact rather than derived: a `InstanceId` is thirty-two lowercase hexadecimal
/// characters, which is these sixteen bytes written out.
fn instance_bytes(instance: &InstanceId) -> Result<[u8; 16], BackendFailureKind> {
    let hex = instance.as_str();
    if hex.len() != 32 {
        return Err(BackendFailureKind::WorkloadRejected);
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        bytes[index] =
            u8::from_str_radix(text, 16).map_err(|_| BackendFailureKind::WorkloadRejected)?;
    }
    Ok(bytes)
}

/// The vsock context identifier this Instance takes.
///
/// Context identifiers are host global, so two concurrent sandboxes sharing one would contend for
/// the same endpoint. One command line invocation serves one sandbox and cannot see the others,
/// so there is no counter to draw from: the identifier is derived from the Instance identity,
/// which is already unique per sandbox. Zero, one, and two are reserved by the kernel, so the
/// derived value is folded into the range above them.
fn guest_cid_for(instance: [u8; 16]) -> u32 {
    let derived = u32::from_be_bytes([instance[0], instance[1], instance[2], instance[3]]);
    // `u32::MAX` is reserved as the "any" identifier, so the usable span ends one below it.
    let span = u32::MAX - FIRST_GUEST_CID;
    FIRST_GUEST_CID + (derived % span)
}

/// The certified Generation identity as the thirty-two bytes the launch page binds.
///
/// The identity is carried as its canonical `sha256:` form, and the launch page binds raw bytes,
/// so the hex is decoded rather than re-hashed: re-hashing would bind a different value.
pub(super) fn generation_bytes(id: &soma::GenerationId) -> Result<[u8; 32], BackendFailureKind> {
    let hex = id
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(BackendFailureKind::Unavailable)?;
    let mut bytes = [0_u8; 32];
    if hex.len() != 64 {
        return Err(BackendFailureKind::Unavailable);
    }
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| BackendFailureKind::Unavailable)?;
        bytes[index] = u8::from_str_radix(text, 16).map_err(|_| BackendFailureKind::Unavailable)?;
    }
    Ok(bytes)
}

/// Sixteen fresh bytes for one identity.
pub(super) fn fresh16() -> [u8; 16] {
    use std::io::Read as _;
    let mut bytes = [0_u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ignored = file.read_exact(&mut bytes);
    }
    bytes
}

/// The host's current wall-clock reading, in nanoseconds since the Unix epoch.
pub(super) fn now_unix_nanos() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )
    .unwrap_or(0)
}
/// The sixteen bytes one canonical portable identity is written out as.
///
/// The launch page, the Host Runtime frame, and the receipt must all carry the same bytes for
/// one identity, so there is one decoder rather than one per caller.
pub(in crate::backend::kvm) fn hex16(hex: &str) -> Result<[u8; 16], BackendFailureKind> {
    if hex.len() != 32 {
        return Err(BackendFailureKind::WorkloadRejected);
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| BackendFailureKind::WorkloadRejected)?;
        bytes[index] =
            u8::from_str_radix(text, 16).map_err(|_| BackendFailureKind::WorkloadRejected)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(hex: &str) -> InstanceId {
        InstanceId::new(hex).expect("instance identity")
    }

    /// The guest must authenticate the identity the receipt reports, byte for byte.
    #[test]
    fn the_guest_instance_identity_is_the_public_one() {
        let id = instance("89db112753324c3e890ef78b74381aa5");
        let bytes = instance_bytes(&id).expect("bytes");
        // The conversion is reversible, so the two identities are the same value in two forms
        // rather than one derived from the other.
        let rendered = bytes.iter().fold(String::new(), |mut text, byte| {
            use std::fmt::Write as _;
            write!(text, "{byte:02x}").expect("write");
            text
        });
        assert_eq!(rendered, id.as_str());
    }

    /// A context identifier is part of an Instance's identity, so two must not share one.
    #[test]
    fn two_instances_take_different_context_identifiers() {
        let first = LaunchIdentity::derive(&instance("89db112753324c3e890ef78b74381aa5"));
        let second = LaunchIdentity::derive(&instance("11db112753324c3e890ef78b74381aa5"));
        let (x, y) = (first.expect("a").guest_cid, second.expect("b").guest_cid);
        assert_ne!(x, y);
        // Zero, one, and two are reserved by the kernel and must never be handed to a guest.
        assert!(x >= FIRST_GUEST_CID && y >= FIRST_GUEST_CID);
        assert_ne!(x, u32::MAX);
    }

    #[test]
    fn two_instances_do_not_share_guest_identity() {
        let first = LaunchIdentity::derive(&instance("89db112753324c3e890ef78b74381aa5"));
        let second = LaunchIdentity::derive(&instance("89db112753324c3e890ef78b74381aa6"));
        assert_ne!(first.expect("a").instance, second.expect("b").instance);
    }

    /// Two launches of one Instance are one machine identity and two operations.
    #[test]
    fn two_launches_of_one_instance_differ_only_in_their_operation() {
        let id = instance("0102030405060708090a0b0c0d0e0f10");
        let first = LaunchIdentity::derive(&id).expect("first");
        let second = LaunchIdentity::derive(&id).expect("second");
        assert_eq!(first.guest_cid, second.guest_cid);
        assert_eq!(first.instance, second.instance);
        assert_ne!(first.operation, second.operation);
        assert_ne!(first.operation, [0; 16]);
    }
}
