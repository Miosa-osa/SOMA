use soma_guest::CaptureWarmPlan;

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

/// The initramfs layout version produced and accepted by this module.
///
/// Version 2 added the console and null device nodes the kernel opens for PID 1 before
/// devtmpfs is mounted, alongside a Generation-scoped responder private key.
/// Version 3 removes that key and its `etc/soma` directory: the responder static secret is
/// now fresh for every Instance and reaches the guest only through the non-snapshot launch
/// page, so a reusable Generation artifact carries public identity only.
pub const INITRAMFS_LAYOUT_VERSION: u16 = 3;
/// The layout version of an archive that also carries a capture warm plan.
///
/// Version 4 is version 3 plus one read-only [`CAPTURE_WARM_PATH`] entry holding the canonical
/// plan bytes. A Generation that declares no plan keeps version 3 byte for byte, so its
/// initramfs digest, and every digest that binds it, is unchanged.
pub const INITRAMFS_WARM_LAYOUT_VERSION: u16 = 4;
/// The capture warm plan path inside a layout v4 archive.
pub const CAPTURE_WARM_PATH: &str = "warm";
/// The fixed modification time of every initramfs entry.
pub const INITRAMFS_MTIME: u32 = 0;
/// The early-init executable path inside the archive.
pub const EARLY_INIT_PATH: &str = "init";
/// The guest-agent executable path inside the archive.
pub const GUEST_AGENT_PATH: &str = "bin/soma-guest-agent";

const MAGIC: &[u8; 6] = b"070701";
const TRAILER: &str = "TRAILER!!!";
const HEADER_LEN: usize = 110;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;
const S_IFCHR: u32 = 0o020_000;
const MAX_ENTRIES: usize = 64;

/// One allowlisted entry: raw path, mode, and character-device major and minor numbers.
type Layout = (&'static str, u32, (u32, u32));

/// The complete allowlisted entry set in raw path-byte order.
const LAYOUT_V3: &[Layout] = &[
    ("bin", S_IFDIR | 0o755, (0, 0)),
    (GUEST_AGENT_PATH, S_IFREG | 0o755, (0, 0)),
    ("dev", S_IFDIR | 0o755, (0, 0)),
    ("dev/console", S_IFCHR | 0o600, (5, 1)),
    ("dev/null", S_IFCHR | 0o666, (1, 3)),
    (EARLY_INIT_PATH, S_IFREG | 0o755, (0, 0)),
    ("lower", S_IFDIR | 0o755, (0, 0)),
    ("newroot", S_IFDIR | 0o755, (0, 0)),
    ("overlay", S_IFDIR | 0o755, (0, 0)),
    ("proc", S_IFDIR | 0o755, (0, 0)),
    ("sys", S_IFDIR | 0o755, (0, 0)),
];

/// The entry a layout v4 archive adds after every v3 entry; `warm` sorts after `sys`.
const WARM_ENTRY: Layout = (CAPTURE_WARM_PATH, S_IFREG | 0o444, (0, 0));

/// The verified contents of one deterministic initramfs.
///
/// The archive holds two executables and, in layout v4, one canonical capture warm plan; no
/// entry carries a secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitramfsContents {
    /// The digest of the early-init executable bytes.
    pub early_init_digest: Sha256Digest,
    /// The digest of the guest-agent executable bytes.
    pub guest_agent_digest: Sha256Digest,
    /// The layout version the archive was verified against.
    pub layout_version: u16,
    /// The capture warm plan a layout v4 archive carries.
    pub capture_warm: Option<CaptureWarmPlan>,
}

/// The layout version an archive with or without a capture warm plan has.
#[must_use]
pub const fn layout_version(capture_warm: bool) -> u16 {
    if capture_warm {
        INITRAMFS_WARM_LAYOUT_VERSION
    } else {
        INITRAMFS_LAYOUT_VERSION
    }
}

fn layout(capture_warm: bool) -> impl Iterator<Item = &'static Layout> {
    LAYOUT_V3.iter().chain(capture_warm.then_some(&WARM_ENTRY))
}

/// The thirteen fixed-width `newc` header fields in archive order.
type Fields = [u32; 13];

fn fields(inode: u32, mode: u32, rdev: (u32, u32), size: u32, name_len: usize) -> Fields {
    let nlink = if mode & S_IFDIR != 0 { 2 } else { 1 };
    let name_size = u32::try_from(name_len + 1).unwrap_or(u32::MAX);
    [
        inode,
        mode,
        0,
        0,
        nlink,
        INITRAMFS_MTIME,
        size,
        0,
        0,
        rdev.0,
        rdev.1,
        name_size,
        0,
    ]
}

/// Builds the deterministic `newc` archive: layout v3, or layout v4 when a capture warm plan
/// is declared.
///
/// Entries are emitted in raw path-byte order with root ownership, fixed modes, zero mtime,
/// sequential inode numbers, zero device numbers except the two character nodes, zero
/// padding, and a final `TRAILER!!!`.
///
/// # Errors
///
/// Returns [`CompileErrorKind::LimitExceeded`] when the total exceeds `max_bytes`.
pub fn build_initramfs(
    early_init: &[u8],
    guest_agent: &[u8],
    capture_warm: Option<&CaptureWarmPlan>,
    max_bytes: u64,
) -> Result<Vec<u8>, CompileError> {
    let warm = capture_warm.map(CaptureWarmPlan::encode);
    let mut archive = Vec::new();
    for (index, (path, mode, rdev)) in layout(warm.is_some()).enumerate() {
        let body: &[u8] = match *path {
            EARLY_INIT_PATH => early_init,
            GUEST_AGENT_PATH => guest_agent,
            CAPTURE_WARM_PATH => warm.as_deref().unwrap_or_default(),
            _ => &[],
        };
        let inode = u32::try_from(index + 1).map_err(|_| build_limit())?;
        let size = u32::try_from(body.len()).map_err(|_| build_limit())?;
        push_entry(
            &mut archive,
            &fields(inode, *mode, *rdev, size, path.len()),
            path.as_bytes(),
            body,
        );
    }
    push_entry(&mut archive, &TRAILER_FIELDS, TRAILER.as_bytes(), &[]);
    pad(&mut archive, 512);
    if u64::try_from(archive.len()).map_err(|_| build_limit())? > max_bytes {
        return Err(build_limit());
    }
    Ok(archive)
}

const TRAILER_FIELDS: Fields = [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 11, 0];

fn push_entry(archive: &mut Vec<u8>, fields: &Fields, name: &[u8], body: &[u8]) {
    archive.extend_from_slice(MAGIC);
    for field in fields {
        archive.extend_from_slice(format!("{field:08x}").as_bytes());
    }
    archive.extend_from_slice(name);
    archive.push(0);
    pad(archive, 4);
    archive.extend_from_slice(body);
    pad(archive, 4);
}

fn pad(archive: &mut Vec<u8>, alignment: usize) {
    let remainder = archive.len() % alignment;
    if remainder != 0 {
        archive.resize(archive.len() + alignment - remainder, 0);
    }
}

mod reader;

pub use reader::verify_initramfs;

const fn invalid() -> CompileError {
    CompileError::new(
        CompilePhase::VerifyInitramfs,
        CompileErrorKind::InvalidInput,
    )
}

const fn build_limit() -> CompileError {
    CompileError::new(
        CompilePhase::BuildInitramfs,
        CompileErrorKind::LimitExceeded,
    )
}
