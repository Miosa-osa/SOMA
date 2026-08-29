use super::InstancePsk;
use crate::Error;

#[test]
fn instance_psk_debug_output_never_exposes_secret_bytes() {
    let psk = InstancePsk::provision_for([2; 16], [0xAB; 32]).expect("instance PSK");

    assert_eq!(format!("{psk:?}"), "InstancePsk([REDACTED])");
    assert!(!format!("{psk:?}").contains("171"));
}

#[test]
fn zero_instance_psk_material_is_rejected() {
    assert_eq!(
        InstancePsk::provision_for([2; 16], [0; 32]).expect_err("zero PSK"),
        Error::InvalidKeyMaterial
    );
    assert_eq!(
        InstancePsk::provision_for([0; 16], [5; 32]).expect_err("zero Instance"),
        Error::InvalidKeyMaterial
    );
}
