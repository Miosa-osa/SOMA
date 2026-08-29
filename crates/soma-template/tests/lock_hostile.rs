//! The decoder never panics and accepts only canonical bytes.

mod support;

use soma_template::{LOCK_MAGIC, LockError, TemplateLock, WireError};
use support::{EXAMPLE, edit, example, lock, replace_bytes};

#[test]
fn every_prefix_is_rejected_without_panic() {
    let bytes = example().encode();
    for length in 0..bytes.len() {
        let error = TemplateLock::decode(&bytes[..length]).expect_err("short bytes");
        assert!(
            !matches!(error, LockError::Wire(WireError::TrailingBytes(_))),
            "prefix of {length} bytes cannot report trailing bytes"
        );
    }
}

#[test]
fn every_single_bit_flip_never_panics_and_only_canonical_bytes_decode() {
    let bytes = example().encode();
    let mut accepted = 0_usize;
    for index in 0..bytes.len() {
        for bit in 0..8 {
            let mut flipped = bytes.clone();
            flipped[index] ^= 1 << bit;
            if let Ok(lock) = TemplateLock::decode(&flipped) {
                accepted += 1;
                assert_eq!(
                    lock.encode(),
                    flipped,
                    "accepted input at byte {index} bit {bit} must re-encode identically"
                );
            }
        }
    }
    assert!(
        accepted > 0,
        "some content flips are valid alternative locks"
    );
}

#[test]
fn trailing_bytes_are_rejected() {
    let mut bytes = example().encode();
    bytes.push(0);
    assert_eq!(
        TemplateLock::decode(&bytes),
        Err(LockError::Wire(WireError::TrailingBytes(1)))
    );
}

#[test]
fn magic_and_schema_version_are_checked_first() {
    assert_eq!(
        TemplateLock::decode(b""),
        Err(LockError::Wire(WireError::ShortInput {
            needed: 8,
            available: 0
        }))
    );
    let mut bytes = example().encode();
    bytes[0] = b'X';
    assert_eq!(TemplateLock::decode(&bytes), Err(LockError::BadMagic));
    let mut bytes = example().encode();
    bytes[9] = 2;
    assert_eq!(
        TemplateLock::decode(&bytes),
        Err(LockError::UnsupportedLockSchema(2))
    );
    let mut bytes = example().encode();
    bytes[14] = b'X';
    assert!(matches!(
        TemplateLock::decode(&bytes),
        Err(LockError::UnsupportedTemplateSchema(schema)) if schema.starts_with('X')
    ));
}

#[test]
fn absurd_lengths_are_bound_violations_not_allocations() {
    let mut bytes = LOCK_MAGIC.to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(
        TemplateLock::decode(&bytes),
        Err(LockError::Wire(WireError::LengthExceedsBound { .. }))
    ));
}

#[test]
fn invalid_discriminants_and_presence_bytes_are_rejected() {
    let bytes = example().encode();
    let mut seen_discriminant = false;
    let mut seen_presence = false;
    for index in 0..bytes.len() {
        let mut mutated = bytes.clone();
        mutated[index] = 0xff;
        match TemplateLock::decode(&mutated) {
            Err(LockError::InvalidDiscriminant { .. }) => seen_discriminant = true,
            Err(LockError::Wire(WireError::InvalidPresence(0xff))) => seen_presence = true,
            _ => {}
        }
    }
    assert!(seen_discriminant);
    assert!(seen_presence);
}

#[test]
fn random_bytes_never_panic() {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for length in [1_usize, 7, 8, 9, 10, 16, 64, 640, 4096] {
        for _ in 0..64 {
            let bytes: Vec<u8> = (0..length)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    u8::try_from(state & 0xff).expect("masked")
                })
                .collect();
            let _ = TemplateLock::decode(&bytes);
            let mut prefixed = LOCK_MAGIC.to_vec();
            prefixed.extend_from_slice(&1_u16.to_be_bytes());
            prefixed.extend_from_slice(&bytes);
            let _ = TemplateLock::decode(&prefixed);
        }
    }
}

#[test]
fn unsorted_or_duplicate_environment_and_secret_names_are_rejected() {
    let text = edit(
        EXAMPLE,
        "[[secrets]]",
        "[[environment]]\nname = \"AAX_ONE\"\nvalue = \"1\"\n\n[[environment]]\nname = \"AAX_TWO\"\nvalue = \"1\"\n\n[[secrets]]\nname = \"SAX_ONE\"\nsource = \"secret://vault/one\"\ndelivery = \"environment\"\n\n[[secrets]]\nname = \"SAX_TWO\"\nsource = \"secret://vault/two\"\ndelivery = \"environment\"\n\n[[secrets]]",
    );
    let bytes = lock(&text).encode();
    assert!(TemplateLock::decode(&bytes).is_ok());
    let duplicate = replace_bytes(
        &bytes,
        "\x00\x00\x00\x07AAX_ONE",
        "\x00\x00\x00\x07AAX_TWO",
        false,
    );
    assert_eq!(
        TemplateLock::decode(&duplicate),
        Err(LockError::InvalidField {
            field: "environment"
        })
    );
    let reversed = replace_bytes(
        &replace_bytes(
            &bytes,
            "\x00\x00\x00\x07AAX_ONE",
            "\x00\x00\x00\x07AAX_TWO",
            false,
        ),
        "\x00\x00\x00\x07AAX_TWO",
        "\x00\x00\x00\x07AAX_ONE",
        true,
    );
    assert_eq!(
        TemplateLock::decode(&reversed),
        Err(LockError::InvalidField {
            field: "environment"
        })
    );
    let duplicate = replace_bytes(
        &bytes,
        "\x00\x00\x00\x07SAX_ONE",
        "\x00\x00\x00\x07SAX_TWO",
        false,
    );
    assert_eq!(
        TemplateLock::decode(&duplicate),
        Err(LockError::InvalidField { field: "secrets" })
    );
}
