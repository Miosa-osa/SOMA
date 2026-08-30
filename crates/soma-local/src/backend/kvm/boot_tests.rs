//! What the launch inputs must bind, and what they must never share.

use super::*;

/// The guest must authenticate the identity the receipt reports, byte for byte.
#[test]
fn the_guest_instance_identity_is_the_public_one() {
    let instance = InstanceId::new("89db112753324c3e890ef78b74381aa5").expect("identity");
    let bytes = instance_bytes(&instance).expect("bytes");
    assert_eq!(
        bytes,
        [
            0x89, 0xdb, 0x11, 0x27, 0x53, 0x32, 0x4c, 0x3e, 0x89, 0x0e, 0xf7, 0x8b, 0x74, 0x38,
            0x1a, 0xa5
        ]
    );
    // The conversion is reversible, so the two identities are the same value in two forms
    // rather than one derived from the other.
    let rendered = bytes.iter().fold(String::new(), |mut text, byte| {
        use std::fmt::Write as _;
        write!(text, "{byte:02x}").expect("write");
        text
    });
    assert_eq!(rendered, instance.as_str());
}

/// Context identifiers are host global, so two concurrent sandboxes must not share one.
#[test]
fn two_instances_take_different_context_identifiers() {
    let a = InstanceId::new("89db112753324c3e890ef78b74381aa5").expect("a");
    let b = InstanceId::new("11db112753324c3e890ef78b74381aa5").expect("b");
    let (x, y) = (guest_cid_for(&a).expect("a"), guest_cid_for(&b).expect("b"));
    assert_ne!(x, y);
    // Zero, one, and two are reserved by the kernel and must never be handed to a guest.
    assert!(x >= FIRST_GUEST_CID && y >= FIRST_GUEST_CID);
}

#[test]
fn two_instances_do_not_share_guest_identity() {
    let a = InstanceId::new("89db112753324c3e890ef78b74381aa5").expect("a");
    let b = InstanceId::new("89db112753324c3e890ef78b74381aa6").expect("b");
    assert_ne!(
        instance_bytes(&a).expect("a bytes"),
        instance_bytes(&b).expect("b bytes")
    );
}
