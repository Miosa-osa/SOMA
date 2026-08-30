use std::{fs, path::Path};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::{ArtifactRole, Sha256Digest},
    erofs::store_file,
    error::{CompileError, CompileErrorKind, CompilePhase},
    manifest::OverlayTemplate,
    process::{ToolOutcome, version_line},
    request::CompilerProfile,
    toolchain::BuilderEnvironment,
};
use crate::store::Store;

mod tools;
mod verify;

/// The pinned `e2fsprogs` release.
pub const E2FSPROGS_REVISION: &str = "1.47.0";
/// The UUID and hash-seed derivation policy version.
pub const OVERLAY_UUID_DERIVATION_VERSION: u16 = 1;
/// The fixed volume label.
pub const OVERLAY_VOLUME_LABEL: &str = "SOMA_OVERLAY";
/// The exact ext4 feature set every template must report.
pub const OVERLAY_FEATURES: &[&str] = &[
    "has_journal",
    "ext_attr",
    "resize_inode",
    "dir_index",
    "filetype",
    "extent",
    "64bit",
    "flex_bg",
    "sparse_super",
    "large_file",
    "huge_file",
    "dir_nlink",
    "extra_isize",
    "metadata_csum",
];
const UUID_DOMAIN: &[u8] = b"soma-overlay-template-uuid-v1\0";
const SEED_DOMAIN: &[u8] = b"soma-overlay-template-hash-seed-v1\0";
const MKE2FS_CONFIG: &str = "[defaults]\n\tbase_features = sparse_super,large_file,filetype,\
resize_inode,dir_index,ext_attr\n\tdefault_mntopts = acl,user_xattr\n\tenable_periodic_fsck = 0\n\
\tblocksize = 4096\n\tinode_size = 256\n\tinode_ratio = 16384\n\n[fs_types]\n\text4 = {\n\
\t\tfeatures = has_journal,extent,huge_file,flex_bg,metadata_csum,64bit,dir_nlink,extra_isize\n\
\t\tinode_size = 256\n\t}\n";

/// Retained evidence from one overlay-template class build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayClassEvidence {
    /// The exact capacity built.
    pub capacity: u64,
    /// The `mke2fs` invocation.
    pub format: ToolOutcome,
    /// The two `debugfs` directory creations.
    pub populate: Vec<ToolOutcome>,
    /// The read-only `e2fsck -fn` run.
    pub check: ToolOutcome,
    /// The `dumpe2fs -h` and `debugfs ls` inspections.
    pub inspect: Vec<ToolOutcome>,
}

/// Retained evidence from the complete overlay-template build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverlayEvidence {
    /// Every tool this build ran, bound by the digest of the exact executable that ran.
    pub tools: BuilderEnvironment,
    /// The digest of the `mke2fs` executable that ran.
    pub formatter_digest: Sha256Digest,
    /// The reported `e2fsprogs` revision.
    pub revision: String,
    /// One evidence record per capacity class.
    pub classes: Vec<OverlayClassEvidence>,
}

/// Returns the canonical feature-profile string bound into the manifest.
#[must_use]
pub fn overlay_feature_profile() -> String {
    format!(
        "ext4/blk4096/inode256/reserved0/{}/lazy-init-off/root-owner-0:0/dirs-upper-work",
        OVERLAY_FEATURES.join(",")
    )
}

/// Derives the fixed template UUID for one capacity.
#[must_use]
pub fn derive_overlay_uuid(capacity: u64) -> [u8; 16] {
    derive(UUID_DOMAIN, capacity)
}

/// Derives the fixed directory hash seed for one capacity.
#[must_use]
pub fn derive_overlay_hash_seed(capacity: u64) -> [u8; 16] {
    derive(SEED_DOMAIN, capacity)
}

fn derive(domain: &[u8], capacity: u64) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(capacity.to_be_bytes());
    let output = hasher.finalize();
    let mut uuid = [0_u8; 16];
    uuid.copy_from_slice(&output[..16]);
    uuid[6] = (uuid[6] & 0x0f) | 0x40;
    uuid[8] = (uuid[8] & 0x3f) | 0x80;
    uuid
}

/// Creates, verifies, and stores one sterile template per profile capacity class.
///
/// `mke2fs` formats an empty filesystem under a pinned configuration file and fake time, then
/// `debugfs` creates the empty `upper` and `work` directories under the same fake time.
/// This deviates from populating through `mke2fs -d` because that path copies host inode
/// change times and was measured to differ across seconds.
pub(crate) fn compile_overlay_templates(
    e2fsprogs: &Path,
    profile: &CompilerProfile,
    store: &Store,
    staging: &Path,
) -> Result<(Vec<OverlayTemplate>, OverlayEvidence), CompileError> {
    let pinned = tools::PinnedTools::open(e2fsprogs)?;
    let revision = version_line(&pinned.formatter, "-V", staging, CompilePhase::BuildOverlay)?;
    if revision.split(' ').nth(1) != Some(E2FSPROGS_REVISION) {
        return Err(toolchain(CompilePhase::BuildOverlay));
    }
    let bound = pinned.bind(&revision)?;
    let formatter_digest = pinned.formatter.digest();
    let config = staging.join("mke2fs.conf");
    fs::write(&config, MKE2FS_CONFIG).map_err(|_| io_error())?;
    let tools = tools::Tools {
        pinned: &pinned,
        environment: vec![
            ("E2FSPROGS_FAKE_TIME".to_owned(), profile.epoch.to_string()),
            (
                "MKE2FS_CONFIG".to_owned(),
                config.to_string_lossy().into_owned(),
            ),
        ],
        staging,
        profile,
    };
    let mut templates = Vec::new();
    let mut classes = Vec::new();
    for (index, capacity) in profile.overlay_capacities.iter().copied().enumerate() {
        let image = staging.join(format!("overlay-{index}.ext4"));
        let evidence = tools.build_class(capacity, &image)?;
        let descriptor = store_file(&image, ArtifactRole::OverlayTemplate, capacity, store)?;
        if descriptor.size != capacity {
            return Err(CompileError::new(
                CompilePhase::VerifyOverlay,
                CompileErrorKind::Integrity,
            ));
        }
        templates.push(OverlayTemplate {
            capacity,
            descriptor,
        });
        classes.push(evidence);
    }
    Ok((
        templates,
        OverlayEvidence {
            tools: bound,
            formatter_digest,
            revision,
            classes,
        },
    ))
}

const fn toolchain(phase: CompilePhase) -> CompileError {
    CompileError::new(phase, CompileErrorKind::Toolchain)
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::BuildOverlay, CompileErrorKind::Io)
}
