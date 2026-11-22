use std::{
    error::Error,
    ffi::OsStr,
    fs::{self, File},
    io::{self, Read as _},
    os::unix::fs::MetadataExt as _,
    path::{Path, PathBuf},
};

use rustix::fs::{CWD, FlockOperation, RenameFlags, flock, renameat_with};
use soma::{GenerationId, OciDigest, OciImage};
use soma_generation::{CompilerProfile, Sha256Digest, installed_policy_version, verify_generation};

pub(super) fn install(
    root: &OsStr,
    stage: &OsStr,
    reference: &OsStr,
    generation: &OsStr,
) -> Result<PathBuf, Box<dyn Error>> {
    let stage = stage.to_str().ok_or("the stage name must be text")?;
    if stage.is_empty()
        || stage.starts_with('.')
        || !stage
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    {
        return Err("the stage name must be a single directory name".into());
    }
    let reference = OciImage::parse(reference.to_str().ok_or("the reference must be text")?)?;
    let (repository, digest) = reference
        .as_str()
        .rsplit_once('@')
        .filter(|(repository, _)| !repository.is_empty() && !repository.contains('@'))
        .ok_or("the reference must be pinned to an OCI digest")?;
    OciImage::parse(repository)?;
    let digest = OciDigest::parse(digest)?;
    let id = GenerationId::new(
        generation
            .to_str()
            .ok_or("the Generation ID must be text")?,
    )?;

    let root = Path::new(root);
    let owner = directory(root)?.uid();
    let root = root.canonicalize()?;
    let incoming = root.join(".incoming");
    let incoming_metadata = directory(&incoming)?;
    if incoming_metadata.uid() != owner || incoming_metadata.mode() & 0o077 != 0 {
        return Err(".incoming must be private and owned by the prepared root's owner".into());
    }

    let staged = incoming.join(stage);
    let key = Sha256Digest::of(reference.as_str().as_bytes()).to_string();
    let key = key.strip_prefix("sha256:").expect("canonical digest");
    let target = root.join(format!("ref-{key}"));

    // A directory lock keeps publication serialized even when an installer exits or crashes.
    let lock = File::open(&incoming)?;
    flock(&lock, FlockOperation::LockExclusive)?;
    let installed = existing_claim(&root, reference.as_str())?;
    let Some(entry) = installed else {
        verify_entry(&staged, owner, &reference, &id, &digest)?;
        validate_tree(&staged, owner, true)?;
        renameat_with(CWD, &staged, CWD, &target, RenameFlags::NOREPLACE)?;
        File::open(&root)?.sync_all()?;
        lock.sync_all()?;
        return Ok(target);
    };

    // An identical retry is harmless, including when the first attempt already moved the stage.
    verify_entry(&entry, owner, &reference, &id, &digest)?;
    File::open(&root)?.sync_all()?;
    lock.sync_all()?;
    Ok(entry)
}

fn directory(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::other(
            "expected a protected, non-symlink directory",
        ));
    }
    Ok(metadata)
}

fn existing_claim(root: &Path, reference: &str) -> Result<Option<PathBuf>, Box<dyn Error>> {
    let mut found = None;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err("the prepared root contains a symbolic link".into());
        }
        if !kind.is_dir() || entry.file_name() == ".incoming" {
            continue;
        }
        let path = entry.path();
        match read_identity(&path.join("reference"), 1_024) {
            Ok(observed) if observed.trim() == reference => {
                if found.is_some() {
                    return Err("multiple prepared entries claim the approved reference".into());
                }
                found = Some(path);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(found)
}

fn read_identity(path: &Path, limit: u64) -> io::Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::other("invalid identity file"));
    }
    let mut value = String::new();
    File::open(path)?
        .take(limit + 1)
        .read_to_string(&mut value)?;
    if u64::try_from(value.len()).unwrap_or(u64::MAX) > limit {
        return Err(io::Error::other("identity file exceeds its size limit"));
    }
    Ok(value)
}

fn verify_entry(
    entry: &Path,
    owner: u32,
    reference: &OciImage,
    id: &GenerationId,
    digest: &OciDigest,
) -> Result<(), Box<dyn Error>> {
    validate_tree(entry, owner, false)?;
    if read_identity(&entry.join("reference"), 1_024)? != reference.as_str()
        || read_identity(&entry.join("generation.id"), 71)? != id.as_str()
    {
        return Err("the prepared entry differs from the approved reference or Generation".into());
    }

    let store = entry.join("store");
    directory(&store)?;
    let version = installed_policy_version(&store, id)?;
    let profile = CompilerProfile::from_policy_version(version)
        .ok_or("the Generation's compiler policy is unsupported")?;
    let verified = verify_generation(&store, id, &profile)?;
    if &verified.manifest.source.oci_manifest_digest.to_oci() != digest {
        return Err("the Generation does not contain the pinned OCI image".into());
    }
    Ok(())
}

// Requiring exclusive write authority protects the bytes between verification and rename.
fn validate_tree(entry: &Path, owner: u32, sync: bool) -> Result<(), Box<dyn Error>> {
    let mut directories = vec![entry.to_path_buf()];
    while let Some(path) = directories.pop() {
        if directory(&path)?.uid() != owner {
            return Err("a staged directory has a different owner".into());
        }
        for child in fs::read_dir(&path)? {
            let child = child?.path();
            let metadata = fs::symlink_metadata(&child)?;
            if metadata.is_dir() {
                directories.push(child);
            } else if !metadata.is_file()
                || metadata.uid() != owner
                || metadata.nlink() != 1
                || metadata.mode() & 0o022 != 0
            {
                return Err("the staged entry contains a shared or unprotected file".into());
            } else if sync {
                File::open(&child)?.sync_all()?;
            }
        }
        if sync {
            File::open(&path)?.sync_all()?;
        }
    }
    Ok(())
}
