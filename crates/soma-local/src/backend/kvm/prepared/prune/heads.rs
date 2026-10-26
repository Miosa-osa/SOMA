//! Heads still named on disk, and the Instance each one was made for.
//!
//! A head is cloned and unlinked in one step (`boot::head`), so a head root normally names no head
//! at all: a running machine's head exists only as the descriptor its machine holds, and that
//! machine's Instance record already names its Generation. A head that is still named is one whose
//! unlink never ran. A head records nothing about its Generation; its name is the Instance it was
//! made for, so that name is the only way to tie it to one. The fan directory holds template
//! replicas, which are copies rather than heads, and is not read.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// One head still named under the head root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NamedHead {
    /// The file name, which is the lowercase identity of the Instance it was made for.
    pub(super) name: String,
    pub(super) path: PathBuf,
}

/// Every head named under `root`: in its shards, or directly in it for an unsharded root.
///
/// An absent root names nothing. A root that cannot be read is an error, because a head this
/// could not see may be the one that ties an entry to a live Instance.
pub(super) fn named_heads(root: &Path) -> Result<Vec<NamedHead>, String> {
    let describe = |error: std::io::Error| format!("{}: {error}", root.display());
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(describe(error)),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("{}: the head root is a link", root.display()));
        }
        Ok(_) => {}
    }
    let mut heads = Vec::new();
    for child in std::fs::read_dir(root).map_err(describe)? {
        let child = child.map_err(describe)?;
        let kind = child.file_type().map_err(describe)?;
        let name = child.file_name().to_string_lossy().into_owned();
        if !kind.is_dir() {
            heads.push(NamedHead {
                name,
                path: child.path(),
            });
        } else if is_shard(&name) {
            for head in std::fs::read_dir(child.path()).map_err(describe)? {
                let head = head.map_err(describe)?;
                if !head.file_type().map_err(describe)?.is_dir() {
                    heads.push(NamedHead {
                        name: head.file_name().to_string_lossy().into_owned(),
                        path: head.path(),
                    });
                }
            }
        }
    }
    heads.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(heads)
}

/// Whether `name` is a shard directory: `h` and at least two digits (`soma_storage::shard`).
fn is_shard(name: &str) -> bool {
    name.strip_prefix('h')
        .is_some_and(|digits| digits.len() >= 2 && digits.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_names_are_h_and_two_or_more_digits() {
        assert!(is_shard("h00"));
        assert!(is_shard("h15"));
        assert!(is_shard("h1023"));
        assert!(!is_shard("h1"));
        assert!(!is_shard("fan"));
        assert!(!is_shard("hxx"));
    }

    #[test]
    fn named_heads_are_read_from_shards_and_the_root_but_not_the_fan() {
        let root = tempfile::tempdir().expect("head root");
        std::fs::create_dir_all(root.path().join("h00")).expect("shard");
        std::fs::create_dir_all(root.path().join("h01")).expect("shard");
        std::fs::create_dir_all(root.path().join("fan/abcd/0")).expect("fan");
        std::fs::write(root.path().join("h01/leaked"), b"").expect("head");
        std::fs::write(root.path().join("unsharded"), b"").expect("head");
        std::fs::write(root.path().join("fan/abcd/0/replica"), b"").expect("replica");
        let names: Vec<String> = named_heads(root.path())
            .expect("readable root")
            .into_iter()
            .map(|head| head.name)
            .collect();
        assert_eq!(names, ["leaked", "unsharded"]);
    }

    #[test]
    fn an_absent_head_root_names_nothing() {
        let root = tempfile::tempdir().expect("scratch");
        assert_eq!(named_heads(&root.path().join("absent")), Ok(Vec::new()));
    }
}
