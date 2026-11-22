//! What each sandbox asked for, kept on the host's disk so it outlives soma-api.
//!
//! The controller's list is memory, so an upgrade or a crash used to end every sandbox's sizing
//! and forget every range a user set. With a file named by [`remember_in`], every range and every
//! deliberate turn-off is written as it changes, and read back when soma-api starts. A saved
//! sandbox that no longer runs is dropped at its first reading, like any other.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use soma::InstanceId;

use super::{Policy, default, sandboxes};

/// The file under the state root that holds the saved sizing.
pub const FILE: &str = "autosize.json";

static PATH: OnceLock<PathBuf> = OnceLock::new();
/// Serialises writers, so two changes never interleave their writes.
static WRITING: Mutex<()> = Mutex::new(());

/// The saved form: each sized sandbox's range, and the sandboxes turned off on purpose.
#[derive(Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Saved {
    #[serde(default)]
    pub sized: BTreeMap<String, Policy>,
    #[serde(default)]
    pub off: BTreeSet<String>,
}

/// Keeps the sizing in `path` from now on, and restores what it already holds.
///
/// Returns how many sandboxes were restored.
pub fn remember_in(path: PathBuf) -> usize {
    let saved = read(&path);
    let _ = PATH.set(path);
    let mut restored = 0;
    for (instance, policy) in saved.sized {
        if let Ok(instance) = InstanceId::new(instance) {
            super::set_quiet(&instance, Some(policy));
            restored += 1;
        }
    }
    for instance in saved.off {
        if let Ok(instance) = InstanceId::new(instance) {
            default::turn_off_quiet(&instance);
        }
    }
    restored
}

/// The saved sizing at `path`; an absent or unreadable file is no saved sizing.
#[must_use]
pub fn read(path: &Path) -> Saved {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Writes the sizing as the controller holds it now, when a file was named.
pub fn save() {
    let Some(path) = PATH.get() else {
        return;
    };
    let _writing = WRITING.lock().unwrap_or_else(PoisonError::into_inner);
    let saved = Saved {
        sized: sandboxes()
            .iter()
            .map(|(instance, entry)| (instance.clone(), entry.policy))
            .collect(),
        off: default::turned_off_now(),
    };
    if let Err(error) = write(path, &saved) {
        eprintln!("soma-api: the autosize settings could not be saved: {error}");
    }
}

/// Writes `saved` to `path` through a temporary file, so a crash never leaves half a file.
///
/// # Errors
///
/// Returns the failure to write or rename the file.
pub fn write(path: &Path, saved: &Saved) -> std::io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(saved).map_err(std::io::Error::other)?;
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_sizing_reads_back_as_written() {
        let dir = std::env::temp_dir().join(format!("soma-autosize-saved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        let mut saved = Saved::default();
        saved.sized.insert(
            "sbx-a".to_owned(),
            Policy {
                min_cpu: 1,
                max_cpu: 8,
                min_memory_mb: 2048,
                max_memory_mb: 16384,
            },
        );
        saved.off.insert("sbx-b".to_owned());
        write(&path, &saved).unwrap();
        assert_eq!(read(&path), saved);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_broken_file_is_no_saved_sizing() {
        let dir = std::env::temp_dir().join(format!("soma-autosize-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read(&dir.join("absent.json")), Saved::default());
        std::fs::write(dir.join(FILE), b"{not json").unwrap();
        assert_eq!(read(&dir.join(FILE)), Saved::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
