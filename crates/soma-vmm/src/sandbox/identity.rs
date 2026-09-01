//! The identities one launch binds, derived in one place so two processes cannot disagree.
//!
//! A broker outside the jail claims the network for an Instance and a machine inside the jail
//! is built with it. If each derived its own context identifier, the machine would be built
//! under one identity and given a network leased to another, and no reader of the evidence
//! could tell. So the derivation lives here and both sides call it.

use soma_guest::LaunchNetwork;

/// The lowest context identifier a guest may take; 0, 1, and 2 are reserved by the kernel.
pub const FIRST_GUEST_CID: u32 = 3;

/// The locally administered MAC the guest sees on its one network device.
pub const GUEST_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];

/// The vsock context identifier this Instance takes.
///
/// Context identifiers are host global, so two concurrent sandboxes sharing one would contend
/// for the same endpoint. There is no counter a single sandbox can draw from, so the identifier
/// is derived from the Instance identity, which is already unique per sandbox. Zero, one, and
/// two are reserved by the kernel, so the derived value is folded into the range above them.
#[must_use]
pub fn guest_cid_for(instance: [u8; 16]) -> u32 {
    let derived = u32::from_be_bytes([instance[0], instance[1], instance[2], instance[3]]);
    // `u32::MAX` is reserved as the "any" identifier, so the usable span ends one below it.
    let span = u32::MAX - FIRST_GUEST_CID;
    FIRST_GUEST_CID + (derived % span)
}

/// The link-down placeholder network every guest is given today.
///
/// The addresses are fixed because nothing routes them: the device exists so the guest's repair
/// step has one to configure, and no packet leaves the machine.
///
/// The context identifier is not fixed. The guest agent checks the identifier its own vsock
/// device reports against the one the launch page names, and refuses the session when they
/// disagree, which is what binds the transport the session runs over to this Instance's
/// authority. So this must be given the same identifier the machine was built with rather than
/// a constant: a launch page naming a different one leaves a correctly built machine unable to
/// form a session at all.
///
/// Returns `None` when the values do not form a launch network, which the fixed addresses here
/// cannot produce.
#[must_use]
pub fn link_down_network(guest_cid: u32) -> Option<LaunchNetwork> {
    LaunchNetwork::new(
        guest_cid,
        1,
        GUEST_MAC,
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        now_unix_nanos(),
    )
    .ok()
}

/// The host's current wall-clock reading, in nanoseconds since the Unix epoch.
#[must_use]
pub fn now_unix_nanos() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )
    .unwrap_or(0)
}

/// Sixteen fresh bytes for one identity, taken from the kernel CSPRNG.
///
/// The syscall is used rather than a handle on `/dev/urandom` because this runs inside the jail
/// as well as outside it, and the jail's root holds no device to open.
#[must_use]
#[allow(unsafe_code)]
pub fn fresh16() -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    let mut filled = 0;
    while filled < bytes.len() {
        let remainder = &mut bytes[filled..];
        // SAFETY: the pointer and length describe valid writable storage.
        let taken = unsafe { libc::getrandom(remainder.as_mut_ptr().cast(), remainder.len(), 0) };
        match usize::try_from(taken) {
            Ok(0) => break,
            Ok(count) => filled += count,
            // An interruption is retried; anything else leaves the identity unusable, and the
            // contract types refuse an all-zero identifier rather than accepting a weak one.
            Err(_) if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted => {
            }
            Err(_) => break,
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_instances_take_different_context_identifiers() {
        let first = guest_cid_for([0x89, 0xdb, 0x11, 0x27, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let second = guest_cid_for([0x11, 0xdb, 0x11, 0x27, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_ne!(first, second);
        assert!(first >= FIRST_GUEST_CID && second >= FIRST_GUEST_CID);
        assert_ne!(first, u32::MAX);
    }

    #[test]
    fn fresh_identities_are_not_all_zero_and_not_repeated() {
        let first = fresh16();
        assert_ne!(first, [0; 16]);
        assert_ne!(first, fresh16());
    }
}
