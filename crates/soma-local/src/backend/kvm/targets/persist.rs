//! The persisted desired set: written whole by rename, read at start.

use std::{io::Write as _, path::Path};

use crate::prewarm::{PERSISTED_FILE, PersistedTargets, PrewarmTarget};

/// Reads the persisted set. `Ok(None)` is no file; `Err` is a file that exists and cannot be used.
pub(super) fn load(state_root: &Path) -> Result<Option<Vec<PrewarmTarget>>, String> {
    let path = state_root.join(PERSISTED_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {PERSISTED_FILE}: {}", error.kind())),
    };
    let persisted: PersistedTargets = serde_json::from_slice(&bytes)
        .map_err(|_| format!("{PERSISTED_FILE} is not a valid prewarm set"))?;
    if persisted.version != 1 {
        return Err(format!("{PERSISTED_FILE} has an unknown version"));
    }
    Ok(Some(persisted.targets))
}

/// Writes the set to a temporary file and renames it, so a reader sees the old set or the new one.
pub(super) fn persist(state_root: &Path, targets: &[PrewarmTarget]) -> Result<(), String> {
    let body = serde_json::to_vec_pretty(&PersistedTargets {
        version: 1,
        targets: targets.to_vec(),
    })
    .map_err(|_| "the set could not be encoded".to_owned())?;
    let temporary = state_root.join(format!("{PERSISTED_FILE}.tmp"));
    let _ = std::fs::remove_file(&temporary);
    let result = (|| -> std::io::Result<()> {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&body)?;
        file.sync_all()?;
        std::fs::rename(&temporary, state_root.join(PERSISTED_FILE))?;
        // The rename is durable once the directory is.
        std::fs::File::open(state_root)?.sync_all()
    })();
    result.map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("could not persist the set: {}", error.kind())
    })
}
