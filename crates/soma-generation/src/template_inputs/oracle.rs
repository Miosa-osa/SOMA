//! A filesystem oracle backed by the normalized rootfs of the resolved image.

use std::{io::Read as _, path::Path};

use soma_template::{FilesystemOracle, OracleError, ResolvedImage};

use crate::{
    ImportPhase, NormalizedRootfs, TreeBounds,
    generation::tree_decoder::{TreeDecoder, TreeNode},
    normalize::TREE_MEDIA_TYPE,
    oci::Descriptor,
    store::Store,
    template_inputs::lookup::{self, Node, Tree},
};

/// The directories a bare program name is looked up in.
///
/// A Template may write `program = "claude"` instead of an absolute path, and the guest resolves
/// such a name against `PATH`. The guest agent has no login shell to inherit one from, so these
/// are the six directories of the conventional default `PATH`, in the order a shell searches.
const DEFAULT_PATH: [&str; 6] = [
    "/usr/local/sbin",
    "/usr/local/bin",
    "/usr/sbin",
    "/usr/bin",
    "/sbin",
    "/bin",
];

/// The largest normalized-tree manifest this oracle will read.
const MAX_TREE_MANIFEST_BYTES: u64 = 512 * 1024 * 1024;

/// Answers executable questions about one normalized rootfs.
///
/// The tree is decoded once and held in memory, because a resolution asks about a single
/// program and the alternative is decoding the whole manifest again for every question.
pub struct RootfsOracle {
    manifest_digest: soma::OciDigest,
    tree: Tree,
}

impl RootfsOracle {
    /// Decodes the normalized tree of `rootfs` from the content store at `store`.
    ///
    /// # Errors
    ///
    /// Returns [`OracleError`] when the tree manifest cannot be read from the store or does not
    /// decode within `bounds`.
    pub fn open(
        store: &Path,
        rootfs: &NormalizedRootfs,
        bounds: TreeBounds,
    ) -> Result<Self, OracleError> {
        let bytes = read_tree_manifest(store, rootfs)?;
        let decoder = TreeDecoder::new(&bytes, bounds)
            .map_err(|error| OracleError::new(&format!("normalized tree: {error}")))?;
        let mut tree = Tree::new();
        for entry in decoder {
            let entry = entry
                .map_err(|error| OracleError::new(&format!("normalized tree entry: {error}")))?;
            let executable = entry.mode & 0o111 != 0;
            let node = match entry.node {
                TreeNode::Directory => Node::Directory,
                TreeNode::Regular { .. } | TreeNode::Hardlink { .. } => Node::File { executable },
                TreeNode::Symlink { target } => Node::Symlink { target },
                TreeNode::Fifo => Node::Other,
            };
            tree.insert(entry.path, node);
        }
        Ok(Self {
            manifest_digest: rootfs.workload().manifest_digest().clone(),
            tree,
        })
    }

    fn is_executable(&self, path: &[u8]) -> bool {
        matches!(
            lookup::resolve(&self.tree, path),
            Some(Node::File { executable: true })
        )
    }
}

impl FilesystemOracle for RootfsOracle {
    fn executable_present(
        &self,
        image: &ResolvedImage,
        program: &str,
    ) -> Result<bool, OracleError> {
        // Answering for a different image would be a guess. The caller pairs an oracle with the
        // rootfs of one resolved manifest, so a mismatch is a wiring error, not a missing file.
        if image.digest() != &self.manifest_digest {
            return Err(OracleError::new(
                "oracle holds the rootfs of a different image",
            ));
        }
        if program.starts_with('/') {
            return Ok(self.is_executable(program.as_bytes()));
        }
        if program.contains('/') {
            // A relative path has no defined starting directory before the workload's working
            // directory exists, so it names nothing this oracle can confirm.
            return Ok(false);
        }
        Ok(DEFAULT_PATH
            .iter()
            .any(|directory| self.is_executable(format!("{directory}/{program}").as_bytes())))
    }
}

fn read_tree_manifest(store: &Path, rootfs: &NormalizedRootfs) -> Result<Vec<u8>, OracleError> {
    let descriptor = Descriptor {
        media_type: TREE_MEDIA_TYPE.to_owned(),
        digest: rootfs.tree_manifest_digest().clone(),
        size: rootfs.tree_manifest_size(),
        platform: None,
    };
    let store = Store::open(store)
        .map_err(|error| OracleError::new(&format!("opening the content store: {error}")))?;
    let mut file = store
        .open_verified_blob(&descriptor, MAX_TREE_MANIFEST_BYTES, ImportPhase::Publish)
        .map_err(|error| OracleError::new(&format!("reading the normalized tree: {error}")))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| OracleError::new("reading the normalized tree failed"))?;
    Ok(bytes)
}
