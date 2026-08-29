use std::collections::BTreeMap;

use super::{
    erofs_reader::{ErofsImage, Inode, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFREG},
    error::{CompileError, CompileErrorKind, CompilePhase},
    tree_decoder::{TreeBounds, TreeDecoder, TreeNode},
};

const FT_REG: u8 = 1;
const FT_DIR: u8 = 2;
const FT_FIFO: u8 = 5;
const FT_SYMLINK: u8 = 7;
const MAX_DEPTH: usize = 1024;

/// What the verifier requires of the image beyond tree equality.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootExpectation {
    pub(crate) uuid: [u8; 16],
    pub(crate) volume_name: [u8; 16],
    pub(crate) epoch: u64,
}

/// The result of one independent image traversal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootVerification {
    pub(crate) entry_count: u32,
    pub(crate) inode_count: u64,
}

/// Independently walks an EROFS image and requires exact equality with the canonical tree.
///
/// Method: the crate-private reader parses the superblock, every directory block, every inode,
/// symlink target, and regular-file body without mounting or extracting. Every path, node type,
/// permission bits, numeric owner, modification time, link target, hard-link group and link
/// count, file size, and content digest must match the decoded tree, and the image must contain
/// no additional entry.
pub(crate) fn verify_root_image(
    mut image: ErofsImage,
    manifest: &[u8],
    bounds: TreeBounds,
    expectation: &RootExpectation,
) -> Result<RootVerification, CompileError> {
    let superblock = image.superblock;
    if superblock.uuid != expectation.uuid
        || superblock.volume_name != expectation.volume_name
        || superblock.build_time != expectation.epoch
        || superblock.feature_incompat != 0
    {
        return Err(integrity());
    }
    let mut decoder = TreeDecoder::new(manifest, bounds)?;
    let max_entries = usize::try_from(decoder.declared_entries()).map_err(|_| limit())?;
    let nodes = collect_nodes(&mut image, superblock.root_nid, max_entries)?;
    if nodes.len() != max_entries {
        return Err(integrity());
    }
    let mut groups = BTreeMap::<Vec<u8>, (u64, u32)>::new();
    let mut compared = 0_u32;
    for entry in decoder.by_ref() {
        let entry = entry?;
        let inode = nodes.get(&entry.path).ok_or_else(integrity)?;
        let expected_type = match entry.node {
            TreeNode::Directory => S_IFDIR,
            TreeNode::Regular { .. } | TreeNode::Hardlink { .. } => S_IFREG,
            TreeNode::Symlink { .. } => S_IFLNK,
            TreeNode::Fifo => S_IFIFO,
        };
        let mode = u32::from(inode.mode & !S_IFMT);
        if inode.mode & S_IFMT != expected_type
            || mode != entry.mode
            || inode.uid != entry.uid
            || inode.gid != entry.gid
            || inode.mtime != expectation.epoch
            || inode.xattr_count != 0
        {
            return Err(integrity());
        }
        match &entry.node {
            TreeNode::Directory | TreeNode::Fifo => {
                if inode.size != 0 && expected_type == S_IFIFO {
                    return Err(integrity());
                }
            }
            TreeNode::Regular { size, digest } => {
                if inode.size != *size || image.hash_data(inode)? != *digest {
                    return Err(integrity());
                }
                groups.insert(entry.path.clone(), (inode.nid, 1));
            }
            TreeNode::Symlink { target } => {
                let length = usize::try_from(inode.size).map_err(|_| integrity())?;
                if length != target.len() || image.read_data(inode, 0, length)? != *target {
                    return Err(integrity());
                }
            }
            TreeNode::Hardlink { anchor } => {
                let group = groups.get_mut(anchor).ok_or_else(integrity)?;
                if group.0 != inode.nid {
                    return Err(integrity());
                }
                group.1 = group.1.checked_add(1).ok_or_else(limit)?;
            }
        }
        compared = compared.checked_add(1).ok_or_else(limit)?;
    }
    let summary = decoder.finish()?;
    for (path, (_, count)) in &groups {
        let inode = nodes.get(path).ok_or_else(integrity)?;
        if inode.nlink != *count {
            return Err(integrity());
        }
    }
    if compared != summary.entry_count {
        return Err(integrity());
    }
    Ok(RootVerification {
        entry_count: compared,
        inode_count: superblock.inode_count,
    })
}

fn collect_nodes(
    image: &mut ErofsImage,
    root_nid: u64,
    max_entries: usize,
) -> Result<BTreeMap<Vec<u8>, Inode>, CompileError> {
    let mut nodes = BTreeMap::new();
    let root = image.inode(root_nid)?;
    if root.mode & S_IFMT != S_IFDIR {
        return Err(integrity());
    }
    nodes.insert(Vec::new(), root);
    let mut stack = vec![(Vec::new(), root, 0_usize)];
    while let Some((path, directory, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(limit());
        }
        for dirent in image.read_dir(&directory, max_entries)? {
            if dirent.name == b"." || dirent.name == b".." {
                if dirent.nid
                    != if dirent.name == b"." {
                        directory.nid
                    } else {
                        parent_nid(&nodes, &path)?
                    }
                {
                    return Err(integrity());
                }
                continue;
            }
            if nodes.len() >= max_entries {
                return Err(integrity());
            }
            let inode = image.inode(dirent.nid)?;
            let expected = match inode.mode & S_IFMT {
                S_IFDIR => FT_DIR,
                S_IFREG => FT_REG,
                S_IFLNK => FT_SYMLINK,
                S_IFIFO => FT_FIFO,
                _ => return Err(unsupported()),
            };
            if dirent.file_type != expected {
                return Err(integrity());
            }
            let mut child = path.clone();
            if !child.is_empty() {
                child.push(b'/');
            }
            child.extend_from_slice(&dirent.name);
            if nodes.insert(child.clone(), inode).is_some() {
                return Err(integrity());
            }
            if inode.mode & S_IFMT == S_IFDIR {
                stack.push((child, inode, depth + 1));
            }
        }
    }
    Ok(nodes)
}

fn parent_nid(nodes: &BTreeMap<Vec<u8>, Inode>, path: &[u8]) -> Result<u64, CompileError> {
    let parent = path
        .iter()
        .rposition(|byte| *byte == b'/')
        .map_or(b"".as_slice(), |index| &path[..index]);
    nodes
        .get(parent)
        .map(|inode| inode.nid)
        .ok_or_else(integrity)
}

const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::VerifyRoot, CompileErrorKind::Integrity)
}

const fn unsupported() -> CompileError {
    CompileError::new(CompilePhase::VerifyRoot, CompileErrorKind::Unsupported)
}

const fn limit() -> CompileError {
    CompileError::new(CompilePhase::VerifyRoot, CompileErrorKind::LimitExceeded)
}
