use super::*;

const PAGE_PREFIX_HEX: &str = concat!(
    "534f4d412d4c41554e43482d5041474500010001",
    "0101010101010101010101010101010101010101010101010101010101010101",
    "02020202020202020202020202020202",
    "03030303030303030303030303030303",
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
    "2122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40",
    "4142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60",
    "6162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f80",
);

#[test]
fn canonical_launch_page_matches_the_frozen_v1_vector() {
    let material = HostLaunchMaterial::generate_with([1; 32], [2; 16], [3; 16], |random| {
        for (index, byte) in random.iter_mut().enumerate() {
            *byte = u8::try_from(index + 1).expect("128-byte fixture");
        }
        Ok(())
    })
    .expect("deterministic material");
    let binding = material.binding;
    let mut page = [0xA5; LAUNCH_PAGE_SIZE];
    let delivered = material
        .deliver_with(|encoded| {
            page.copy_from_slice(encoded);
            Ok::<(), ()>(())
        })
        .expect("launch-page delivery");
    let expected_prefix = decode_hex(PAGE_PREFIX_HEX);

    assert_eq!(&page[..wire::ENCODED_SIZE], expected_prefix);
    assert!(page[wire::ENCODED_SIZE..].iter().all(|byte| *byte == 0));
    assert_eq!(delivered.binding(), &binding);
    let prologue = binding.prologue();
    assert_eq!(&page[18..20], &prologue[21..23]);
}

#[test]
fn zero_random_fields_retry_before_material_is_admitted() {
    for zeroed in [0..32, 32..64, 64..128] {
        let mut calls = 0;
        let material = HostLaunchMaterial::generate_with([1; 32], [2; 16], [3; 16], |random| {
            calls += 1;
            random.fill(9);
            if calls == 1 {
                random[zeroed.clone()].fill(0);
            }
            Ok(())
        })
        .expect("second random sample");

        assert_eq!(calls, 2);
        assert_eq!(material.binding.launch_nonce(), &[9; 32]);
    }
}

#[test]
fn repeated_zero_random_material_fails_after_the_bounded_attempts() {
    let mut calls = 0;
    let error = HostLaunchMaterial::generate_with([1; 32], [2; 16], [3; 16], |random| {
        calls += 1;
        random.fill(0);
        Ok(())
    })
    .expect_err("zero random source");

    assert_eq!(calls, RANDOM_ATTEMPTS);
    assert_eq!(error, Error::RandomnessUnavailable);
}

#[test]
fn invalid_caller_identity_is_rejected_before_randomness_is_requested() {
    for (generation, instance, operation) in [
        ([0; 32], [2; 16], [3; 16]),
        ([1; 32], [0; 16], [3; 16]),
        ([1; 32], [2; 16], [0; 16]),
    ] {
        let error = HostLaunchMaterial::generate_with(generation, instance, operation, |_| {
            panic!("random source must not be called")
        })
        .expect_err("zero identity");
        assert_eq!(error, Error::InvalidBinding);
    }
}

fn decode_hex(encoded: &str) -> Vec<u8> {
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|pair| {
            let text = core::str::from_utf8(pair).expect("ASCII hex");
            u8::from_str_radix(text, 16).expect("valid hex")
        })
        .collect()
}
