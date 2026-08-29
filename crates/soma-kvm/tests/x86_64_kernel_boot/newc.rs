//! Minimal deterministic `newc` cpio writer for the kernel-boot fixture.
//!
//! Test-local on purpose: it packs one directory tree of a few entries and is not the
//! production archive writer.

const HEADER_MAGIC: &[u8] = b"070701";
const MODE_DIRECTORY: u32 = 0o040_755;
const MODE_EXECUTABLE: u32 = 0o100_755;
const MODE_CHARACTER_DEVICE: u32 = 0o020_600;
const CONSOLE_MAJOR: u32 = 5;
const CONSOLE_MINOR: u32 = 1;

struct Entry<'a> {
    name: &'a str,
    mode: u32,
    data: &'a [u8],
    rdev: (u32, u32),
}

fn encode(entry: &Entry<'_>, inode: u32) -> Vec<u8> {
    let name = entry.name.as_bytes();
    let name_size = u32::try_from(name.len() + 1).unwrap();
    let file_size = u32::try_from(entry.data.len()).unwrap();
    let fields = [
        inode,
        entry.mode,
        0,
        0,
        1,
        0,
        file_size,
        0,
        0,
        entry.rdev.0,
        entry.rdev.1,
        name_size,
        0,
    ];
    let mut bytes = HEADER_MAGIC.to_vec();
    for field in fields {
        bytes.extend_from_slice(format!("{field:08x}").as_bytes());
    }
    bytes.extend_from_slice(name);
    bytes.push(0);
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    bytes.extend_from_slice(entry.data);
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    bytes
}

/// Packs `/init`, `/dev/console`, and `/proc` into one `newc` archive.
pub fn build_initramfs(init: &[u8]) -> Vec<u8> {
    let entries = [
        Entry {
            name: "dev",
            mode: MODE_DIRECTORY,
            data: &[],
            rdev: (0, 0),
        },
        Entry {
            name: "dev/console",
            mode: MODE_CHARACTER_DEVICE,
            data: &[],
            rdev: (CONSOLE_MAJOR, CONSOLE_MINOR),
        },
        Entry {
            name: "proc",
            mode: MODE_DIRECTORY,
            data: &[],
            rdev: (0, 0),
        },
        Entry {
            name: "init",
            mode: MODE_EXECUTABLE,
            data: init,
            rdev: (0, 0),
        },
        Entry {
            name: "TRAILER!!!",
            mode: 0,
            data: &[],
            rdev: (0, 0),
        },
    ];
    let mut archive = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        archive.extend(encode(entry, u32::try_from(index + 1).unwrap()));
    }
    archive
}

#[test]
fn archive_layout_is_deterministic_and_aligned() {
    let first = build_initramfs(b"\x7fELF");
    let second = build_initramfs(b"\x7fELF");
    assert_eq!(first, second);
    assert!(first.starts_with(b"070701"));
    assert_eq!(first.len() % 4, 0);
    assert!(first.windows(10).any(|w| w == b"TRAILER!!!"));
    let console = encode(
        &Entry {
            name: "dev/console",
            mode: MODE_CHARACTER_DEVICE,
            data: &[],
            rdev: (CONSOLE_MAJOR, CONSOLE_MINOR),
        },
        2,
    );
    assert_eq!(&console[6 + 8 * 9..6 + 8 * 10], b"00000005");
    assert_eq!(&console[6 + 8 * 10..6 + 8 * 11], b"00000001");
}
