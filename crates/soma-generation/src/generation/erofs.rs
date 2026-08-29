use std::{ffi::OsString, fs::File, path::Path};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    erofs_reader::ErofsImage,
    erofs_verify::{RootExpectation, RootVerification, verify_root_image},
    error::{CompileError, CompileErrorKind, CompilePhase},
    process::{Invocation, ToolOutcome, executable_digest, tool_path, version_line},
    request::CompilerProfile,
    tar_stream::stream_tree,
};
use crate::{ImportPhase, store::Store};

/// The pinned `erofs-utils` release.
pub const EROFS_UTILS_REVISION: &str = "1.9.4";
/// The pinned `erofs-utils` commit that release 1.9.4 names.
pub const EROFS_UTILS_COMMIT: &str = "f36cadb5c563995ab3aa8572a60ed6b721b9557d";
/// The immutable EROFS format-profile name bound into the manifest.
pub const EROFS_FORMAT_PROFILE: &str = "erofs/v1/blk4096/uncompressed/no-xattr/tar-full/all-time";
/// The fixed volume label.
pub const EROFS_VOLUME_LABEL: &str = "SOMA_ROOT";
const UUID_DOMAIN: &[u8] = b"soma-erofs-root-uuid-v1\0";
const MKFS: &str = "mkfs.erofs";
const FSCK: &str = "fsck.erofs";

/// Retained evidence from one EROFS build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErofsEvidence {
    /// The digest of the formatter executable that ran.
    pub formatter_digest: Sha256Digest,
    /// The formatter revision reported by the executable.
    pub formatter_revision: String,
    /// The pinned commit the profile requires.
    pub pinned_commit: &'static str,
    /// The formatter invocation.
    pub format: ToolOutcome,
    /// The checker invocation.
    pub check: ToolOutcome,
    /// The derived filesystem UUID.
    pub uuid: [u8; 16],
    /// The independent traversal result.
    pub entries_verified: u32,
    /// The inode count reported by the image superblock.
    pub inode_count: u64,
}

/// Derives the fixed filesystem UUID from the normalized-tree digest.
///
/// The value is the first 16 bytes of `SHA-256("soma-erofs-root-uuid-v1\0" || digest)` with
/// the RFC 4122 version nibble set to 4 and the variant bits set to `10`.
#[must_use]
pub fn derive_root_uuid(tree_digest: &Sha256Digest) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(UUID_DOMAIN);
    hasher.update(tree_digest.as_bytes());
    let output = hasher.finalize();
    let mut uuid = [0_u8; 16];
    uuid.copy_from_slice(&output[..16]);
    uuid[6] = (uuid[6] & 0x0f) | 0x40;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    uuid
}

/// Formats a UUID in the canonical hyphenated lowercase form.
#[must_use]
pub fn format_uuid(uuid: &[u8; 16]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(32);
    for byte in uuid {
        write!(hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

pub(crate) fn volume_name() -> [u8; 16] {
    let mut name = [0_u8; 16];
    name[..EROFS_VOLUME_LABEL.len()].copy_from_slice(EROFS_VOLUME_LABEL.as_bytes());
    name
}

/// Formats, checks, independently verifies, and stores the immutable EROFS root.
pub(crate) fn compile_root(
    erofs_utils: &Path,
    profile: &CompilerProfile,
    tree_manifest: &[u8],
    tree_digest: &Sha256Digest,
    store: &Store,
    staging: &Path,
) -> Result<(ArtifactDescriptor, ErofsEvidence), CompileError> {
    let mkfs = tool_path(erofs_utils, MKFS);
    let fsck = tool_path(erofs_utils, FSCK);
    let formatter_revision = require_revision(&mkfs, staging)?;
    require_revision(&fsck, staging)?;
    let formatter_digest = executable_digest(&mkfs, CompilePhase::FormatRoot)?;
    let uuid = derive_root_uuid(tree_digest);
    let image = staging.join("root.erofs");
    let arguments = vec![
        OsString::from("-b4096"),
        OsString::from("-x-1"),
        OsString::from("--workers=1"),
        OsString::from(format!("-T{}", profile.epoch)),
        OsString::from("--all-time"),
        OsString::from(format!("-U{}", format_uuid(&uuid))),
        OsString::from(format!("-L{EROFS_VOLUME_LABEL}")),
        OsString::from("--tar=f"),
        OsString::from("--quiet"),
        image.clone().into_os_string(),
        OsString::from("/dev/stdin"),
    ];
    let format = Invocation {
        program: &mkfs,
        arguments,
        environment: Vec::new(),
        working_directory: staging,
        deadline: profile.tool_deadline,
        phase: CompilePhase::FormatRoot,
    }
    .run_with_stdin(|stdin| {
        stream_tree(
            tree_manifest,
            profile.tree,
            profile.max_stream_bytes,
            store,
            stdin,
        )
        .map(|_| ())
    })?;
    if !format.succeeded() {
        return Err(CompileError::new(
            CompilePhase::FormatRoot,
            CompileErrorKind::Toolchain,
        ));
    }
    let check = Invocation {
        program: &fsck,
        arguments: vec![image.clone().into_os_string()],
        environment: Vec::new(),
        working_directory: staging,
        deadline: profile.tool_deadline,
        phase: CompilePhase::FormatRoot,
    }
    .run()?;
    if !check.succeeded() {
        return Err(CompileError::new(
            CompilePhase::FormatRoot,
            CompileErrorKind::Toolchain,
        ));
    }
    let expectation = RootExpectation {
        uuid,
        volume_name: volume_name(),
        epoch: profile.epoch,
    };
    let RootVerification {
        entry_count,
        inode_count,
    } = verify_root_image(
        ErofsImage::open(&image, profile.max_root_bytes)?,
        tree_manifest,
        profile.tree,
        &expectation,
    )?;
    let descriptor = store_file(
        &image,
        ArtifactRole::ErofsRoot,
        profile.max_root_bytes,
        store,
    )?;
    Ok((
        descriptor,
        ErofsEvidence {
            formatter_digest,
            formatter_revision,
            pinned_commit: EROFS_UTILS_COMMIT,
            format,
            check,
            uuid,
            entries_verified: entry_count,
            inode_count,
        },
    ))
}

fn require_revision(program: &Path, staging: &Path) -> Result<String, CompileError> {
    let line = version_line(program, "-V", staging, CompilePhase::FormatRoot)?;
    let revision = line
        .rsplit(' ')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    if revision != EROFS_UTILS_REVISION {
        return Err(CompileError::new(
            CompilePhase::FormatRoot,
            CompileErrorKind::Toolchain,
        ));
    }
    Ok(revision)
}

/// Streams one staged file into the store under its digest and returns its descriptor.
pub(crate) fn store_file(
    path: &Path,
    role: ArtifactRole,
    max_bytes: u64,
    store: &Store,
) -> Result<ArtifactDescriptor, CompileError> {
    let mut file = File::open(path)
        .map_err(|_| CompileError::new(CompilePhase::Publish, CompileErrorKind::Io))?;
    let size = file
        .metadata()
        .map_err(|_| CompileError::new(CompilePhase::Publish, CompileErrorKind::Io))?
        .len();
    let descriptor = store
        .put_content(
            &mut file,
            size,
            max_bytes,
            role.media_type(),
            ImportPhase::Publish,
        )
        .map_err(|error| CompileError::from_import(CompilePhase::Publish, error))?;
    Ok(ArtifactDescriptor {
        role,
        digest: Sha256Digest::from_oci(&descriptor.digest),
        size: descriptor.size,
    })
}
