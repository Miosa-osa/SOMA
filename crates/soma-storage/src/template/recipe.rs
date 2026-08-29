//! Pure derivation of the pinned `mke2fs` and `e2fsck` invocations for an overlay recipe.
//!
//! Everything here is deterministic text; the subprocesses run in the parent module.

use std::path::Path;

use sha2::{Digest, Sha256};

use crate::profile::{InodePolicy, OverlayRecipe, UuidPolicy};

/// Fixed creation time written into every template, `2026-08-29T00:00:00Z`.
pub const FAKE_TIME_EPOCH: u64 = 1_787_961_600;

/// Minimal `mke2fs.conf` that pins every default the command line does not name.
///
/// The system configuration is never consulted, so a distribution change cannot alter the
/// template bytes.
pub const MKE2FS_CONFIG: &str = "[defaults]\n\
    \tbase_features = none\n\
    \tdefault_mntopts = acl,user_xattr\n\
    \tenable_periodic_fsck = 0\n\
    \tblocksize = 4096\n\
    \tinode_size = 256\n\
    \tinode_ratio = 16384\n\
    \treserved_ratio = 0\n\
    \tlazy_itable_init = 0\n\
    \n\
    [fs_types]\n\
    \text4 = {\n\
    \t\tfeatures = none\n\
    \t\tinode_size = 256\n\
    \t}\n\
    \tfloppy = {\n\
    \t\tinode_ratio = 16384\n\
    \t}\n\
    \tsmall = {\n\
    \t\tinode_ratio = 16384\n\
    \t}\n";

/// `PATH` given to the formatter so the pinned distribution tools are found without the
/// caller's environment.
pub const TOOL_PATH: &str = "/usr/sbin:/sbin:/usr/bin:/bin";

/// A 16-byte identifier rendered in the canonical `8-4-4-4-12` form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedUuid([u8; 16]);

impl DerivedUuid {
    /// The raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Canonical lowercase text.
    #[must_use]
    pub fn render(&self) -> String {
        let b = self.0;
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
             {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15]
        )
    }
}

/// Derives the filesystem UUID and directory hash seed for a recipe.
///
/// Both come from one SHA-256 over a domain string, the class name, version, and logical size,
/// with the RFC 9562 version 8 and variant bits set so the values are well-formed UUIDs.
#[must_use]
pub fn derive_identity(recipe: &OverlayRecipe) -> (DerivedUuid, DerivedUuid) {
    let mut hasher = Sha256::new();
    hasher.update(b"SOMA overlay class identity v1\0");
    hasher.update(recipe.name.as_str().as_bytes());
    hasher.update([0u8]);
    hasher.update(recipe.version.to_le_bytes());
    hasher.update(recipe.logical_bytes.get().to_le_bytes());
    let digest = hasher.finalize();
    let mut fs_uuid = [0u8; 16];
    fs_uuid.copy_from_slice(&digest[..16]);
    let mut hash_seed = [0u8; 16];
    hash_seed.copy_from_slice(&digest[16..]);
    let fs_uuid = match recipe.uuid_policy {
        UuidPolicy::Derived => stamp_uuid_bits(fs_uuid),
        UuidPolicy::Explicit(explicit) => explicit,
    };
    (
        DerivedUuid(fs_uuid),
        DerivedUuid(stamp_uuid_bits(hash_seed)),
    )
}

fn stamp_uuid_bits(mut bytes: [u8; 16]) -> [u8; 16] {
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

/// One exact subprocess invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// Program name resolved through [`TOOL_PATH`].
    pub program: &'static str,
    /// Complete argument vector after the program name.
    pub args: Vec<String>,
    /// Complete environment; the caller's environment is discarded.
    pub env: Vec<(String, String)>,
}

/// The exact `mke2fs` invocation for `recipe` targeting `image` with `config` as its
/// `mke2fs.conf`.
#[must_use]
pub fn mke2fs_invocation(recipe: &OverlayRecipe, image: &Path, config: &Path) -> Invocation {
    let (fs_uuid, hash_seed) = derive_identity(recipe);
    let InodePolicy::BytesPerInode(bytes_per_inode) = recipe.inode_policy;
    let extended = format!(
        "hash_seed={},lazy_itable_init=0,lazy_journal_init=0,root_owner=0:0,nodiscard",
        hash_seed.render()
    );
    let args = vec![
        "-F".to_owned(),
        "-q".to_owned(),
        "-t".to_owned(),
        "ext4".to_owned(),
        "-b".to_owned(),
        recipe.block_size.bytes().to_string(),
        "-I".to_owned(),
        "256".to_owned(),
        "-i".to_owned(),
        bytes_per_inode.to_string(),
        "-m".to_owned(),
        "0".to_owned(),
        "-r".to_owned(),
        "1".to_owned(),
        "-L".to_owned(),
        volume_label(recipe),
        "-U".to_owned(),
        fs_uuid.render(),
        "-e".to_owned(),
        "remount-ro".to_owned(),
        "-O".to_owned(),
        recipe.features.mke2fs_argument().to_owned(),
        "-E".to_owned(),
        extended,
        image.to_string_lossy().into_owned(),
    ];
    let env = vec![
        ("PATH".to_owned(), TOOL_PATH.to_owned()),
        (
            "E2FSPROGS_FAKE_TIME".to_owned(),
            FAKE_TIME_EPOCH.to_string(),
        ),
        (
            "MKE2FS_CONFIG".to_owned(),
            config.to_string_lossy().into_owned(),
        ),
        ("LC_ALL".to_owned(), "C".to_owned()),
    ];
    Invocation {
        program: "mke2fs",
        args,
        env,
    }
}

/// ext4 volume label: the class name cut to the 16-byte label limit.
#[must_use]
pub fn volume_label(recipe: &OverlayRecipe) -> String {
    let name = recipe.name.as_str();
    name.chars().take(16).collect()
}

/// The read-only forced `e2fsck` check that must pass before a template is trusted.
#[must_use]
pub fn e2fsck_invocation(image: &Path) -> Invocation {
    Invocation {
        program: "e2fsck",
        args: vec![
            "-f".to_owned(),
            "-n".to_owned(),
            image.to_string_lossy().into_owned(),
        ],
        env: vec![
            ("PATH".to_owned(), TOOL_PATH.to_owned()),
            (
                "E2FSPROGS_FAKE_TIME".to_owned(),
                FAKE_TIME_EPOCH.to_string(),
            ),
            ("LC_ALL".to_owned(), "C".to_owned()),
        ],
    }
}

/// File name of the template for a recipe inside a template store.
#[must_use]
pub fn template_file_name(recipe: &OverlayRecipe) -> String {
    format!("{}-v{}.ext4", recipe.name.as_str(), recipe.version)
}
