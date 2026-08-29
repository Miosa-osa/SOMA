use soma_guest::{
    Error, ResponderKeypair, ResponderPrivateKey, ResponderPublicKey, SessionBinding,
};

#[test]
fn generated_keys_cross_an_explicit_provisioning_boundary() {
    let generated = ResponderKeypair::generate().expect("generated keypair");
    let public =
        ResponderPublicKey::new(generated.public_key().to_bytes()).expect("persisted public key");
    let private_bytes = generated
        .private_key()
        .expose_for_provisioning(|secret| *secret);
    let private = ResponderPrivateKey::new(private_bytes).expect("provisioned private key");

    assert_eq!(public.to_bytes(), generated.public_key().to_bytes());
    assert_eq!(format!("{private:?}"), "ResponderPrivateKey([REDACTED])");
}

#[test]
fn secret_debug_output_and_errors_never_include_key_bytes() {
    let private = ResponderPrivateKey::new([0xCD; 32]).expect("private key");

    assert_eq!(format!("{private:?}"), "ResponderPrivateKey([REDACTED])");
    assert_eq!(
        format!("{:?}", Error::AuthenticationFailed),
        "peer authentication failed"
    );
}

#[test]
fn zero_key_and_identity_material_is_rejected() {
    assert_eq!(
        ResponderPrivateKey::new([0; 32]).expect_err("zero private key"),
        Error::InvalidKeyMaterial
    );
    assert_eq!(
        ResponderPublicKey::new([0; 32]).expect_err("zero public key"),
        Error::InvalidKeyMaterial
    );
    assert_eq!(
        SessionBinding::new([0; 32], [2; 16], [3; 16], [4; 32]),
        Err(Error::InvalidBinding)
    );
}

#[test]
fn non_contributory_x25519_public_keys_are_rejected() {
    for (ordinal, public) in non_contributory_public_keys().into_iter().enumerate() {
        let expected = if ordinal == 0 {
            Error::InvalidKeyMaterial
        } else {
            Error::NonContributoryPublicKey
        };
        assert_eq!(
            ResponderPublicKey::new(public).expect_err("low-order key"),
            expected
        );
    }
}

fn non_contributory_public_keys() -> [[u8; 32]; 7] {
    // Frozen X25519 low-order corpus from curve25519-dalek's published constants.
    [
        [0; 32],
        low_value(0x01),
        [
            0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f,
            0xc4, 0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16,
            0x5f, 0x49, 0xb8, 0x00,
        ],
        [
            0x5f, 0x9c, 0x95, 0xbc, 0xa3, 0x50, 0x8c, 0x24, 0xb1, 0xd0, 0xb1, 0x55, 0x9c, 0x83,
            0xef, 0x5b, 0x04, 0x44, 0x5c, 0xc4, 0x58, 0x1c, 0x8e, 0x86, 0xd8, 0x22, 0x4e, 0xdd,
            0xd0, 0x9f, 0x11, 0x57,
        ],
        near_modulus(0xec),
        near_modulus(0xed),
        near_modulus(0xee),
    ]
}

const fn low_value(first: u8) -> [u8; 32] {
    let mut value = [0; 32];
    value[0] = first;
    value
}

const fn near_modulus(first: u8) -> [u8; 32] {
    let mut value = [0xff; 32];
    value[0] = first;
    value[31] = 0x7f;
    value
}
