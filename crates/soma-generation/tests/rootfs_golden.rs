mod support;

use std::fs;

use support::rootfs::{TarEntry, normalize_layers, tar};

const GOLDEN_HEX: &str = concat!(
    "534f4d41524653000001000100000002",
    "0000000001000001ed0000000000000000000000000000000000000000",
    "000000016102000001a40000000000000000000000000000000000000000",
    "00000000000000012d711642b726b04401627ca9fbac32f5c8530fb1903c",
    "c4db02258717921a4881"
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
        "sha256:1db3b547222fcd28b6866dc24eb90e85f04873d2be86af4a8d585852b22fed9f"
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
