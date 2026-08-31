mod support;

use std::fs;

use support::rootfs::{TarEntry, normalize_layers, tar};

const GOLDEN_HEX: &str = concat!(
    "534f4d415246530000010001000000070000000001000001ed000000",
    "0000000000000000000000000000000000000000016102000001a400",
    "0000000000000000000000000000000000000000000000000000012d",
    "711642b726b04401627ca9fbac32f5c8530fb1903cc4db0225871792",
    "1a48810000000364657601000001ed00000000000000000000000000",
    "000000000000000000000470726f6301000001ed0000000000000000",
    "0000000000000000000000000000000372756e01000001ed00000000",
    "000000000000000000000000000000000000000373797301000001ed",
    "000000000000000000000000000000000000000000000003746d7001",
    "000001ed0000000000000000000000000000000000000000"
);

#[test]
fn canonical_tree_manifest_has_pinned_bytes_and_sha256_identity() {
    let layer = tar(&[TarEntry::file(b"a", b"x")]);
    let (fixture, normalized) = normalize_layers(&[layer]);
    let bytes = fs::read(
        fixture
            .store
            .join("v1/blobs/sha256")
            .join(&normalized.tree_manifest_digest().as_str()[7..]),
    )
    .unwrap();

    assert_eq!(bytes, decode_hex(GOLDEN_HEX));
    assert_eq!(
        normalized.tree_manifest_digest().as_str(),
        "sha256:9f43eb05a828aecf5693924ef718b6a09efebf8ffb523b5368ccbfb4edbe60af"
    );
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("golden hex is lowercase ASCII"),
    }
}
