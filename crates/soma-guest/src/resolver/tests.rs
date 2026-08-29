use snow::{
    Error as SnowError,
    params::{DHChoice, NoiseParams},
    resolvers::CryptoResolver,
};

use crate::NOISE_PATTERN;

use super::{ContributoryResolver, noise_builder};

// Cacophony vector distributed with Snow 0.10.0 under Snow's Apache-2.0 OR MIT license.
const PROLOGUE: &str = "4a6f686e2047616c74";
const PSK: &str = "54686973206973206d7920417573747269616e20706572737065637469766521";
const INIT_EPHEMERAL: &str = "893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a";
const RESP_STATIC: &str = "4a3acbfdb163dec651dfa3194dece676d437029c62a408b4c5ea9114246e4893";
const RESP_PUBLIC: &str = "31e0303fd6418d2f8c0e78b91f22e8caed0fbe48656dcf4767e4834f701b8f62";
const RESP_EPHEMERAL: &str = "bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b";
const FIRST_PAYLOAD: &str = "4c756477696720766f6e204d69736573";
const FIRST_CIPHERTEXT: &str = concat!(
    "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c7944",
    "27635ede06947b2d3acd77a36788aaaf17e9f5a8ac252e560fb421ba161a2cf8",
);
const SECOND_PAYLOAD: &str = "4d757272617920526f746862617264";
const SECOND_CIPHERTEXT: &str = concat!(
    "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f144808843",
    "d682eb9cf4fee6816c8c8cfd34c15774321e234e3a426d7cfd3f13e5e84d04",
);
const HANDSHAKE_HASH: &str = "6bd69bd4066f41f32e47134976f5bf01606f7a4a0e04369fe61158b06f3a144e";

#[test]
fn fixed_profile_matches_the_cacophony_nkpsk0_vector() {
    let params: NoiseParams = NOISE_PATTERN.parse().expect("fixed Noise profile");
    let prologue = decode(PROLOGUE);
    let psk = decode_array(PSK);
    let init_ephemeral = decode_array::<32>(INIT_EPHEMERAL);
    let resp_static = decode_array::<32>(RESP_STATIC);
    let resp_public = decode_array::<32>(RESP_PUBLIC);
    let resp_ephemeral = decode_array::<32>(RESP_EPHEMERAL);
    let mut initiator = noise_builder(params.clone())
        .psk(0, &psk)
        .expect("PSK")
        .remote_public_key(&resp_public)
        .expect("responder public key")
        .fixed_ephemeral_key_for_testing_only(&init_ephemeral)
        .prologue(&prologue)
        .expect("prologue")
        .build_initiator()
        .expect("initiator");
    let mut responder = noise_builder(params)
        .psk(0, &psk)
        .expect("PSK")
        .local_private_key(&resp_static)
        .expect("responder private key")
        .fixed_ephemeral_key_for_testing_only(&resp_ephemeral)
        .prologue(&prologue)
        .expect("prologue")
        .build_responder()
        .expect("responder");

    exchange_and_assert(
        &mut initiator,
        &mut responder,
        FIRST_PAYLOAD,
        FIRST_CIPHERTEXT,
    );
    exchange_and_assert(
        &mut responder,
        &mut initiator,
        SECOND_PAYLOAD,
        SECOND_CIPHERTEXT,
    );
    assert_eq!(initiator.get_handshake_hash(), decode(HANDSHAKE_HASH));
    assert_eq!(responder.get_handshake_hash(), decode(HANDSHAKE_HASH));
}

#[test]
fn wrapped_dh_rejects_known_non_contributory_public_values() {
    let resolver = ContributoryResolver::default();
    let mut dh = resolver
        .resolve_dh(&DHChoice::Curve25519)
        .expect("fixed resolver provides X25519");
    dh.set(&[0xA5; 32]);

    for public in non_contributory_public_keys() {
        let mut output = [0_u8; 32];
        assert_eq!(dh.dh(&public, &mut output), Err(SnowError::Dh));
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

fn exchange_and_assert(
    sender: &mut snow::HandshakeState,
    receiver: &mut snow::HandshakeState,
    payload_hex: &str,
    ciphertext_hex: &str,
) {
    let payload = decode(payload_hex);
    let expected = decode(ciphertext_hex);
    let mut ciphertext = vec![0_u8; expected.len()];
    let written = sender
        .write_message(&payload, &mut ciphertext)
        .expect("vector encryption");
    assert_eq!(&ciphertext[..written], expected);
    let mut plaintext = vec![0_u8; payload.len()];
    let read = receiver
        .read_message(&ciphertext[..written], &mut plaintext)
        .expect("vector decryption");
    assert_eq!(&plaintext[..read], payload);
}

fn decode_array<const N: usize>(encoded: &str) -> [u8; N] {
    decode(encoded).try_into().expect("fixed-size vector field")
}

fn decode(encoded: &str) -> Vec<u8> {
    assert_eq!(encoded.len() % 2, 0, "hex vector must contain pairs");
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "hex vector must contain pairs");
    pairs
        .iter()
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

fn nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => panic!("invalid frozen hex vector"),
    }
}
