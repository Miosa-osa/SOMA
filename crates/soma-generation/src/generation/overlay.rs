use std::{ffi::OsString, fs, path::Path};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::{ArtifactRole, Sha256Digest},
    erofs::{format_uuid, store_file},
    error::{CompileError, CompileErrorKind, CompilePhase},
    manifest::OverlayTemplate,
    process::{Invocation, ToolOutcome, executable_digest, tool_path, version_line},
    request::CompilerProfile,
};
use crate::store::Store;

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
    let mke2fs = tool_path(e2fsprogs, "mke2fs");
    let revision = version_line(&mke2fs, "-V", staging, CompilePhase::BuildOverlay)?;
    if revision.split(' ').nth(1) != Some(E2FSPROGS_REVISION) {
        return Err(toolchain(CompilePhase::BuildOverlay));
    }
    let formatter_digest = executable_digest(&mke2fs, CompilePhase::BuildOverlay)?;
    let config = staging.join("mke2fs.conf");
    fs::write(&config, MKE2FS_CONFIG).map_err(|_| io_error())?;
    let tools = Tools {
        directory: e2fsprogs,
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
            formatter_digest,
            revision,
            classes,
        },
    ))
}

struct Tools<'a> {
    directory: &'a Path,
    environment: Vec<(String, String)>,
    staging: &'a Path,
    profile: &'a CompilerProfile,
}

impl Tools<'_> {
    fn run(
        &self,
        program: &str,
        arguments: Vec<OsString>,
        phase: CompilePhase,
    ) -> Result<ToolOutcome, CompileError> {
        Invocation {
            program: &tool_path(self.directory, program),
            arguments,
            environment: self.environment.clone(),
            working_directory: self.staging,
            deadline: self.profile.tool_deadline,
            phase,
        }
        .run()
    }

    fn build_class(
        &self,
        capacity: u64,
        image: &Path,
    ) -> Result<OverlayClassEvidence, CompileError> {
        fs::File::create(image)
            .and_then(|file| file.set_len(capacity))
            .map_err(|_| io_error())?;
        let build = CompilePhase::BuildOverlay;
        let check_phase = CompilePhase::VerifyOverlay;
        let format = self.run("mke2fs", mke2fs_arguments(capacity, image), build)?;
        let populate = vec![
            self.run("debugfs", debugfs(image, true, "mkdir upper"), build)?,
            self.run("debugfs", debugfs(image, true, "mkdir work"), build)?,
        ];
        if !format.succeeded() || populate.iter().any(|outcome| !outcome.succeeded()) {
            return Err(toolchain(build));
        }
        let check = self.run("e2fsck", vec!["-fn".into(), image.into()], check_phase)?;
        let inspect = vec![
            self.run("dumpe2fs", vec!["-h".into(), image.into()], check_phase)?,
            self.run("debugfs", debugfs(image, false, "ls -l /"), check_phase)?,
            self.run(
                "debugfs",
                debugfs(image, false, "ls -l /upper"),
                check_phase,
            )?,
            self.run("debugfs", debugfs(image, false, "ls -l /work"), check_phase)?,
        ];
        verify::verify_class(capacity, &check, &inspect)?;
        Ok(OverlayClassEvidence {
            capacity,
            format,
            populate,
            check,
            inspect,
        })
    }
}

fn mke2fs_arguments(capacity: u64, image: &Path) -> Vec<OsString> {
    let extended = format!(
        "hash_seed={},lazy_itable_init=0,lazy_journal_init=0,root_owner=0:0",
        format_uuid(&derive_overlay_hash_seed(capacity))
    );
    [
        "-F", "-q", "-t", "ext4", "-b", "4096", "-I", "256", "-m", "0", "-U",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([
        OsString::from(format_uuid(&derive_overlay_uuid(capacity))),
        "-L".into(),
        OVERLAY_VOLUME_LABEL.into(),
        "-E".into(),
        extended.into(),
        "-O".into(),
        OVERLAY_FEATURES.join(",").into(),
        image.as_os_str().to_owned(),
    ])
    .collect()
}

fn debugfs(image: &Path, write: bool, request: &str) -> Vec<OsString> {
    let mut arguments = Vec::new();
    if write {
        arguments.push(OsString::from("-w"));
    }
    arguments.push("-R".into());
    arguments.push(request.into());
    arguments.push(image.as_os_str().to_owned());
    arguments
}

const fn toolchain(phase: CompilePhase) -> CompileError {
    CompileError::new(phase, CompileErrorKind::Toolchain)
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::BuildOverlay, CompileErrorKind::Io)
}
