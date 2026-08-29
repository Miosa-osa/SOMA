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

/// The verified contents of one deterministic initramfs.
///
/// The archive holds exactly two byte bodies, both executables; no entry carries a secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitramfsContents {
    /// The digest of the early-init executable bytes.
    pub early_init_digest: Sha256Digest,
    /// The digest of the guest-agent executable bytes.
    pub guest_agent_digest: Sha256Digest,
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

/// Builds the deterministic `newc` archive for layout v3.
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
    max_bytes: u64,
) -> Result<Vec<u8>, CompileError> {
    let mut archive = Vec::new();
    for (index, (path, mode, rdev)) in LAYOUT_V3.iter().enumerate() {
        let body: &[u8] = match *path {
            EARLY_INIT_PATH => early_init,
            GUEST_AGENT_PATH => guest_agent,
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

/// Decodes and verifies a layout v3 archive, rejecting any deviation from the allowlist.
///
/// A layout v2 archive is rejected because its `etc/soma/responder.key` entry is not in the
/// v3 allowlist, so a Generation carrying an immutable guest secret cannot be verified here.
///
/// # Errors
///
/// Returns [`CompileErrorKind::InvalidInput`] for malformed headers, ordering, padding,
/// metadata, unknown paths, or trailing bytes.
pub fn verify_initramfs(archive: &[u8]) -> Result<InitramfsContents, CompileError> {
    let mut cursor = 0_usize;
    let mut expected = LAYOUT_V3.iter().enumerate();
    let mut early_init = None;
    let mut guest_agent = None;
    for _ in 0..=MAX_ENTRIES {
        let entry = read_entry(archive, cursor)?;
        cursor = entry.next;
        if entry.name == TRAILER.as_bytes() {
            if expected.next().is_some() || entry.fields != TRAILER_FIELDS {
                return Err(invalid());
            }
            let trailing = archive.get(cursor..).ok_or_else(invalid)?;
            if trailing.len() >= 512 || trailing.iter().any(|byte| *byte != 0) {
                return Err(invalid());
            }
            return Ok(InitramfsContents {
                early_init_digest: early_init.ok_or_else(invalid)?,
                guest_agent_digest: guest_agent.ok_or_else(invalid)?,
            });
        }
        let (index, (path, mode, rdev)) = expected.next().ok_or_else(invalid)?;
        let inode = u32::try_from(index + 1).map_err(|_| invalid())?;
        let size = u32::try_from(entry.body.len()).map_err(|_| invalid())?;
        if entry.name != path.as_bytes()
            || entry.fields != fields(inode, *mode, *rdev, size, path.len())
        {
            return Err(invalid());
        }
        match *path {
            EARLY_INIT_PATH => early_init = Some(Sha256Digest::of(entry.body)),
            GUEST_AGENT_PATH => guest_agent = Some(Sha256Digest::of(entry.body)),
            _ if !entry.body.is_empty() => return Err(invalid()),
            _ => {}
        }
    }
    Err(invalid())
}

struct RawEntry<'a> {
    fields: Fields,
    name: &'a [u8],
    body: &'a [u8],
    next: usize,
}

fn read_entry(archive: &[u8], start: usize) -> Result<RawEntry<'_>, CompileError> {
    let header = archive
        .get(start..start.checked_add(HEADER_LEN).ok_or_else(invalid)?)
        .ok_or_else(invalid)?;
    if &header[..6] != MAGIC {
        return Err(invalid());
    }
    let mut fields = [0_u32; 13];
    for (index, field) in fields.iter_mut().enumerate() {
        let text =
            std::str::from_utf8(&header[6 + index * 8..14 + index * 8]).map_err(|_| invalid())?;
        if !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid());
        }
        *field = u32::from_str_radix(text, 16).map_err(|_| invalid())?;
    }
    let name_size = usize::try_from(fields[11]).map_err(|_| invalid())?;
    let body_size = usize::try_from(fields[6]).map_err(|_| invalid())?;
    if name_size == 0 || name_size > 256 {
        return Err(invalid());
    }
    let name_start = start + HEADER_LEN;
    let name_end = name_start.checked_add(name_size).ok_or_else(invalid)?;
    let name = archive.get(name_start..name_end).ok_or_else(invalid)?;
    if name[name_size - 1] != 0 || name[..name_size - 1].contains(&0) {
        return Err(invalid());
    }
    let body_start = align4(name_end)?;
    require_zero(archive, name_end, body_start)?;
    let body_end = body_start.checked_add(body_size).ok_or_else(invalid)?;
    let body = archive.get(body_start..body_end).ok_or_else(invalid)?;
    let next = align4(body_end)?;
    require_zero(archive, body_end, next)?;
    Ok(RawEntry {
        fields,
        name: &name[..name_size - 1],
        body,
        next,
    })
}

fn require_zero(archive: &[u8], start: usize, end: usize) -> Result<(), CompileError> {
    let padding = archive.get(start..end).ok_or_else(invalid)?;
    if padding.iter().any(|byte| *byte != 0) {
        return Err(invalid());
    }
    Ok(())
}

fn align4(value: usize) -> Result<usize, CompileError> {
    value.checked_add(3).map(|sum| sum & !3).ok_or_else(invalid)
}

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
