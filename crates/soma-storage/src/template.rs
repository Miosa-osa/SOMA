//! Sterile ext4 template creation with a pinned `mke2fs` subprocess.
//!
//! Template creation is operator-side build work and therefore accepts a template store path.
//! Launch never calls into this module; it only clones a verified template descriptor.

pub mod recipe;

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use sha2::{Digest, Sha256};

use crate::profile::{OverlayRecipe, TemplateDigest};
use recipe::{Invocation, MKE2FS_CONFIG};

/// One created and verified template.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SterileTemplate {
    path: PathBuf,
    digest: TemplateDigest,
    logical_bytes: u64,
    mke2fs: Invocation,
}

impl SterileTemplate {
    /// Location inside the template store.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// SHA-256 of the complete template bytes.
    #[must_use]
    pub fn digest(&self) -> TemplateDigest {
        self.digest
    }

    /// Apparent size in bytes.
    #[must_use]
    pub fn logical_bytes(&self) -> u64 {
        self.logical_bytes
    }

    /// The exact formatter invocation that produced the template.
    #[must_use]
    pub fn mke2fs(&self) -> &Invocation {
        &self.mke2fs
    }

    /// Opens the template read-only.
    ///
    /// # Errors
    ///
    /// Propagates the open failure.
    pub fn open(&self) -> io::Result<File> {
        File::open(&self.path)
    }
}

/// Why template creation failed.
#[derive(Debug)]
pub enum TemplateError {
    /// The template or configuration file could not be created exclusively.
    Create(io::Error),
    /// A tool could not be started.
    Spawn {
        /// Program that failed to start.
        program: &'static str,
        /// The spawn failure.
        error: io::Error,
    },
    /// A tool ran and reported failure.
    Failed {
        /// Program that failed.
        program: &'static str,
        /// Exit status.
        status: ExitStatus,
        /// Captured standard error, bounded by the tool's own output.
        stderr: String,
    },
    /// The template bytes could not be read back for digesting.
    Digest(io::Error),
    /// The created file does not have the requested logical size.
    SizeMismatch {
        /// Requested size.
        expected: u64,
        /// Observed size.
        actual: u64,
    },
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Create(error) => write!(f, "template creation failed: {error}"),
            Self::Spawn { program, error } => write!(f, "{program} could not start: {error}"),
            Self::Failed {
                program,
                status,
                stderr,
            } => {
                write!(f, "{program} failed with {status}: {}", stderr.trim())
            }
            Self::Digest(error) => write!(f, "template digest failed: {error}"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "template has {actual} bytes, expected {expected}")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

/// Creates the template for `recipe` inside `store`, verifies it with `e2fsck -fn`, syncs it,
/// and records its digest.
///
/// The store directory must exist; the template file and its configuration file are created
/// exclusively so an existing template is never overwritten.
///
/// # Errors
///
/// Returns the first failing step; a partially created template is removed.
pub fn create_template(
    store: &Path,
    recipe: &OverlayRecipe,
) -> Result<SterileTemplate, TemplateError> {
    let image = store.join(recipe::template_file_name(recipe));
    let config = store.join(format!(
        "{}-v{}.mke2fs.conf",
        recipe.name.as_str(),
        recipe.version
    ));
    write_exclusive(&config, MKE2FS_CONFIG.as_bytes()).map_err(TemplateError::Create)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&image);
    let result = match file {
        Ok(file) => build(&file, &image, &config, recipe),
        Err(error) => Err(TemplateError::Create(error)),
    };
    let _ = std::fs::remove_file(&config);
    if result.is_err()
        && !matches!(result, Err(TemplateError::Create(ref e)) if e.kind() == io::ErrorKind::AlreadyExists)
    {
        let _ = std::fs::remove_file(&image);
    }
    result
}

fn build(
    file: &File,
    image: &Path,
    config: &Path,
    recipe: &OverlayRecipe,
) -> Result<SterileTemplate, TemplateError> {
    let logical_bytes = recipe.logical_bytes.get();
    file.set_len(logical_bytes).map_err(TemplateError::Create)?;

    let mke2fs = recipe::mke2fs_invocation(recipe, image, config);
    run(&mke2fs)?;
    run(&recipe::e2fsck_invocation(image))?;

    file.sync_all().map_err(TemplateError::Create)?;
    sync_parent(image).map_err(TemplateError::Create)?;
    let actual = file.metadata().map_err(TemplateError::Digest)?.len();
    if actual != logical_bytes {
        return Err(TemplateError::SizeMismatch {
            expected: logical_bytes,
            actual,
        });
    }
    let digest = digest_file(image).map_err(TemplateError::Digest)?;
    Ok(SterileTemplate {
        path: image.to_path_buf(),
        digest,
        logical_bytes,
        mke2fs,
    })
}

fn write_exclusive(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("template has no parent"))?;
    File::open(parent)?.sync_all()
}

fn run(invocation: &Invocation) -> Result<(), TemplateError> {
    let output = Command::new(invocation.program)
        .args(&invocation.args)
        .env_clear()
        .envs(
            invocation
                .env
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| TemplateError::Spawn {
            program: invocation.program,
            error,
        })?;
    if !output.status.success() {
        return Err(TemplateError::Failed {
            program: invocation.program,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

/// Streams a file through SHA-256.
///
/// # Errors
///
/// Propagates read failures.
pub fn digest_file(path: &Path) -> io::Result<TemplateDigest> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&digest);
    Ok(TemplateDigest::from_bytes(bytes))
}

#[cfg(test)]
mod tests;
