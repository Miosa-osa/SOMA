mod support;

use soma_generation::{
    CompileErrorKind, Sha256Digest,
    initramfs::{INITRAMFS_LAYOUT_VERSION, build_initramfs, verify_initramfs},
};

const GOLDEN_HEX: &str = include_str!("fixtures/initramfs_v3.hex");
const INIT: &[u8] = b"#!/bin/sh\nexec /bin/soma-guest-agent\n";
const AGENT: &[u8] = b"synthetic-guest-agent-bytes";

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").unwrap();
        output
    })
}

fn archive() -> Vec<u8> {
    build_initramfs(INIT, AGENT, 1 << 20).unwrap()
}

#[test]
fn initramfs_has_pinned_golden_bytes() {
    let bytes = archive();
    assert_eq!(
        hex(&bytes),
        GOLDEN_HEX.trim(),
        "golden mismatch; actual hex follows\n{}",
        hex(&bytes)
    );
    assert!(bytes.starts_with(b"070701"));
    assert_eq!(bytes.len() % 512, 0);
    assert_eq!(archive(), bytes);
}

#[test]
fn initramfs_round_trips_with_allowlisted_digests() {
    let contents = verify_initramfs(&archive()).unwrap();
    assert_eq!(contents.early_init_digest, Sha256Digest::of(INIT));
    assert_eq!(contents.guest_agent_digest, Sha256Digest::of(AGENT));
}

#[test]
fn initramfs_build_honors_its_byte_bound() {
    assert_eq!(
        build_initramfs(INIT, AGENT, 100).unwrap_err().kind(),
        CompileErrorKind::LimitExceeded
    );
}

#[test]
fn initramfs_carries_device_nodes_and_no_guest_secret_entry() {
    let bytes = archive();
    let console = field_offset(&bytes, b"dev/console", 9);
    assert_eq!(&bytes[console..console + 8], b"00000005");
    assert_eq!(&bytes[console + 8..console + 16], b"00000001");
    let null = field_offset(&bytes, b"dev/null", 9);
    assert_eq!(&bytes[null..null + 8], b"00000001");
    assert_eq!(&bytes[null + 8..null + 16], b"00000003");
    assert_eq!(INITRAMFS_LAYOUT_VERSION, 3);
    for retired in [
        b"etc/soma/responder.key".as_slice(),
        b"etc/soma".as_slice(),
        b"responder".as_slice(),
    ] {
        assert!(
            !bytes.windows(retired.len()).any(|window| window == retired),
            "layout v3 must not carry {}",
            String::from_utf8_lossy(retired)
        );
    }
    let entries = bytes
        .windows(6)
        .filter(|window| *window == b"070701")
        .count();
    assert_eq!(entries, 12, "eleven allowlisted entries plus one trailer");
}

#[test]
fn a_layout_v2_archive_carrying_a_responder_key_is_rejected() {
    let v2 = with_extra_entry(&archive(), b"etc/soma/responder.key", &[7_u8; 32]);
    assert_eq!(
        verify_initramfs(&v2).unwrap_err().kind(),
        CompileErrorKind::InvalidInput
    );
}

/// Splices one extra `newc` entry in front of the trailer of an otherwise valid archive.
fn with_extra_entry(archive: &[u8], name: &[u8], body: &[u8]) -> Vec<u8> {
    let trailer = archive
        .windows(10)
        .position(|window| window == b"TRAILER!!!")
        .unwrap()
        - 110;
    let mut entry = Vec::new();
    entry.extend_from_slice(b"070701");
    let fields: [u32; 13] = [
        99,
        0o100_600,
        0,
        0,
        1,
        0,
        u32::try_from(body.len()).unwrap(),
        0,
        0,
        0,
        0,
        u32::try_from(name.len() + 1).unwrap(),
        0,
    ];
    for field in fields {
        entry.extend_from_slice(format!("{field:08x}").as_bytes());
    }
    entry.extend_from_slice(name);
    entry.push(0);
    while entry.len() % 4 != 0 {
        entry.push(0);
    }
    entry.extend_from_slice(body);
    while entry.len() % 4 != 0 {
        entry.push(0);
    }
    let mut spliced = archive[..trailer].to_vec();
    spliced.extend_from_slice(&entry);
    spliced.extend_from_slice(&archive[trailer..]);
    spliced
}

fn field_offset(archive: &[u8], entry_name: &[u8], field: usize) -> usize {
    let name_position = archive
        .windows(entry_name.len() + 1)
        .position(|window| {
            &window[..entry_name.len()] == entry_name && window[entry_name.len()] == 0
        })
        .unwrap();
    name_position - 110 + 6 + field * 8
}

#[test]
fn initramfs_verifier_rejects_every_metadata_deviation() {
    let good = archive();
    let mut corrupted: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut magic = good.clone();
    magic[0] = b'1';
    corrupted.push(("magic", magic));
    let mut mtime = good.clone();
    let offset = field_offset(&good, b"bin", 5);
    mtime[offset + 7] = b'1';
    corrupted.push(("mtime", mtime));
    let mut uid = good.clone();
    let offset = field_offset(&good, b"init", 2);
    uid[offset + 7] = b'1';
    corrupted.push(("uid", uid));
    let mut mode = good.clone();
    let offset = field_offset(&good, b"init", 1);
    mode[offset + 7] = b'7';
    corrupted.push(("mode", mode));
    let mut inode = good.clone();
    let offset = field_offset(&good, b"dev", 0);
    inode[offset + 7] = b'9';
    corrupted.push(("inode", inode));
    let mut renamed = good.clone();
    let position = renamed.windows(4).position(|w| w == b"bin\0").unwrap();
    renamed[position + 2] = b'm';
    corrupted.push(("unknown path", renamed));
    let mut padding = good.clone();
    let position = padding.windows(4).position(|w| w == b"bin\0").unwrap();
    padding[position + 4] = 1;
    corrupted.push(("name padding", padding));
    let mut trailing = good.clone();
    trailing.push(1);
    corrupted.push(("trailing byte", trailing));
    let mut truncated = good.clone();
    truncated.truncate(truncated.len() - 600);
    corrupted.push(("missing trailer", truncated));
    let mut extra = good.clone();
    let trailer = extra.windows(10).position(|w| w == b"TRAILER!!!").unwrap() - 110;
    let second = good[..field_offset(&good, b"dev", 0) - 6].to_vec();
    extra.splice(trailer..trailer, second[..0].iter().copied());
    let mut duplicated = good[..trailer].to_vec();
    duplicated.extend_from_slice(&good[..field_offset(&good, b"dev", 0) - 6]);
    duplicated.extend_from_slice(&good[trailer..]);
    corrupted.push(("out of order entry", duplicated));
    for (name, bytes) in corrupted {
        assert_eq!(
            verify_initramfs(&bytes).unwrap_err().kind(),
            CompileErrorKind::InvalidInput,
            "{name} was accepted"
        );
    }
    assert!(verify_initramfs(&[]).is_err());
    assert!(verify_initramfs(&good[..good.len() / 2]).is_err());
}
