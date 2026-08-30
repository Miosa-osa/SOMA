//! Transactional publication of one prepared-store entry.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

/// A private sibling directory that becomes visible only after its contents are complete.
#[derive(Debug)]
pub(super) struct Publication {
    final_path: PathBuf,
    staging_path: Option<PathBuf>,
}

impl Publication {
    pub(super) fn begin(final_path: &Path) -> io::Result<Self> {
        let parent = final_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "entry needs a parent directory",
            )
        })?;
        let name = final_path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "entry needs a final name")
        })?;
        fs::create_dir_all(parent)?;
        if fs::symlink_metadata(final_path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "prepared entry already exists; publish a new immutable entry instead",
            ));
        }

        let sequence = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
        let staging_path = parent.join(format!(
            ".{}.prepare-{}-{sequence}",
            name.to_string_lossy(),
            std::process::id()
        ));
        fs::create_dir(&staging_path)?;
        #[cfg(target_os = "linux")]
        set_private_directory(&staging_path)?;
        Ok(Self {
            final_path: final_path.to_path_buf(),
            staging_path: Some(staging_path),
        })
    }

    pub(super) fn path(&self) -> &Path {
        self.staging_path.as_deref().expect("publication is active")
    }

    pub(super) fn write_private(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path().join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(target_os = "linux")]
        options.mode(0o600);
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }

    pub(super) fn commit(mut self) -> io::Result<()> {
        let staging = self.staging_path.take().expect("publication is active");
        rename_noreplace(&staging, &self.final_path)?;
        sync_parent(&self.final_path)
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        if let Some(staging) = self.staging_path.take() {
            let _ = fs::remove_dir_all(staging);
        }
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        from,
        rustix::fs::CWD,
        to,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

#[cfg(not(target_os = "linux"))]
fn rename_noreplace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "prepared Generation publication requires Linux",
    ))
}

#[cfg(target_os = "linux")]
fn set_private_directory(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "entry needs a parent directory",
        )
    })?;
    fs::File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "soma-publication-{label}-{}-{}",
            std::process::id(),
            NEXT_STAGING.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn an_existing_entry_is_never_removed_or_replaced() {
        let root = scratch("existing");
        let entry = root.join("entry");
        fs::create_dir_all(&entry).expect("create existing entry");
        fs::write(entry.join("sentinel"), b"original").expect("write sentinel");

        let error = Publication::begin(&entry).expect_err("existing entry must refuse");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(entry.join("sentinel")).unwrap(), b"original");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn dropping_an_uncommitted_publication_removes_its_staging_directory() {
        let root = scratch("drop");
        fs::create_dir_all(&root).expect("create root");
        let entry = root.join("entry");
        let staging = {
            let publication = Publication::begin(&entry).expect("begin publication");
            let staging = publication.path().to_path_buf();
            publication
                .write_private("reference", b"node:22")
                .expect("write private file");
            staging
        };

        assert!(!staging.exists());
        assert!(!entry.exists());
        fs::remove_dir_all(root).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn commit_publishes_complete_private_bytes_once() {
        let root = scratch("commit");
        fs::create_dir_all(&root).expect("create root");
        let entry = root.join("entry");
        let publication = Publication::begin(&entry).expect("begin publication");
        publication
            .write_private("reference", b"node:22")
            .expect("write reference");

        publication.commit().expect("commit publication");

        assert_eq!(fs::read(entry.join("reference")).unwrap(), b"node:22");
        let mode = fs::metadata(entry.join("reference"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(root).ok();
    }
}
