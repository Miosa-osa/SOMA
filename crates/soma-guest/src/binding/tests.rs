use crate::NOISE_PATTERN;

use super::SessionBinding;

const EXPECTED_PROLOGUE_HEX: &str = concat!(
    "534f4d412d47554553542d434f4e54524f4c00",
    "0001",
    "0001",
    "1111111111111111111111111111111111111111111111111111111111111111",
    "22222222222222222222222222222222",
    "33333333333333333333333333333333",
    "4444444444444444444444444444444444444444444444444444444444444444",
);

#[test]
fn canonical_prologue_and_protocol_match_the_frozen_v1_vector() {
    let binding =
        SessionBinding::new([0x11; 32], [0x22; 16], [0x33; 16], [0x44; 32]).expect("valid binding");

    assert_eq!(hex(&binding.prologue()), EXPECTED_PROLOGUE_HEX);
    assert_eq!(NOISE_PATTERN, "Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s");
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0F)]));
    }
    encoded
}
