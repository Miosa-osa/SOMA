//! Path resolution over a decoded normalized tree.

use std::collections::BTreeMap;

/// What one normalized tree entry is, reduced to what a lookup needs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Node {
    Directory,
    /// A regular file or a hard link, with the owner-execute bit of its own mode.
    File {
        executable: bool,
    },
    /// A symbolic link and its raw target bytes.
    Symlink {
        target: Vec<u8>,
    },
    /// A FIFO, which is never a program.
    Other,
}

/// The normalized tree keyed by its canonical relative path, root spelled as the empty path.
pub(super) type Tree = BTreeMap<Vec<u8>, Node>;

/// The most symbolic links one resolution will follow, matching the Linux `SYMLOOP_MAX`.
const MAX_SYMLINKS: u32 = 40;

/// Resolves one absolute guest path, following symbolic links.
///
/// Paths in a normalized tree are relative and canonical, so resolution walks components from
/// the root and rewrites the remainder whenever a component turns out to be a symbolic link.
/// Base images need this: on Debian `/bin` is a link to `usr/bin`, so a tree lookup of the
/// literal `bin/sh` finds nothing at all.
pub(super) fn resolve<'a>(tree: &'a Tree, path: &[u8]) -> Option<&'a Node> {
    let mut resolved: Vec<Vec<u8>> = Vec::new();
    let mut pending: Vec<Vec<u8>> = split(path);
    pending.reverse();
    let mut symlinks = 0_u32;
    while let Some(component) = pending.pop() {
        match component.as_slice() {
            b"." => continue,
            b".." => {
                resolved.pop();
                continue;
            }
            _ => resolved.push(component),
        }
        let node = tree.get(&join(&resolved))?;
        match node {
            Node::Symlink { target } => {
                symlinks += 1;
                if symlinks > MAX_SYMLINKS {
                    return None;
                }
                resolved.pop();
                if target.first() == Some(&b'/') {
                    resolved.clear();
                }
                for part in split(target).into_iter().rev() {
                    pending.push(part);
                }
            }
            // Only a directory can have anything below it, so a non-directory with components
            // still pending is a path that does not exist rather than one that partly does.
            Node::Directory => {}
            _ if !pending.is_empty() => return None,
            _ => {}
        }
    }
    tree.get(&join(&resolved))
}

fn split(path: &[u8]) -> Vec<Vec<u8>> {
    path.split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

fn join(components: &[Vec<u8>]) -> Vec<u8> {
    components.join(&b'/')
}
