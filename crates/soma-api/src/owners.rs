//! Which tenant a sandbox or a volume belongs to on this service (SOMA#222).
//!
//! The engine and the host daemon key sandboxes and volumes by identity alone, so the tenant a
//! request carries in `x-soma-tenant` is recorded here when a create, a fork or an attach first
//! takes an identity, and every later request on that identity is answered only for that tenant.
//! A request from any other tenant reads the resource as absent and changes nothing.
//!
//! The record is one small file per identity under the state root, so it outlives a restart of
//! this service. It is written whole beside its final name and then linked into place, so a
//! reader sees no owner or the whole owner, never a part of one, and two tenants racing for one
//! identity cannot both win. An identity with no record was created before this service
//! recorded owners, or by the runner, which keeps its own owners: it stays reachable as before.

use std::{
    collections::HashMap,
    fs,
    io::{self, Write as _},
    path::PathBuf,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// The directory, inside the service's own one under the state root, the records live in.
pub const DIRECTORY: &str = "owners";

/// The longest identity a record is kept for: a volume or an instance id in hexadecimal.
const MAX_IDENTITY_BYTES: usize = 64;

/// The kind of resource an owner record names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resource {
    Sandbox,
    Volume,
}

impl Resource {
    const fn directory(self) -> &'static str {
        match self {
            Self::Sandbox => "sandboxes",
            Self::Volume => "volumes",
        }
    }
}

/// What one tenant may see of one identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The tenant recorded as the owner.
    Owner,
    /// No owner is recorded.
    Unclaimed,
    /// Another tenant owns it, so this one reads it as absent.
    Other,
}

/// What a claim did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// The identity had no owner, and now this tenant owns it.
    New,
    /// This tenant already owned it.
    Kept,
    /// Another tenant owns it; nothing was changed.
    Refused,
}

/// The owner records of one service.
#[derive(Debug)]
pub struct Owners {
    root: Option<PathBuf>,
    memory: Mutex<HashMap<(Resource, String), String>>,
}

impl Owners {
    /// Records owners as files under `root`, created on first use.
    #[must_use]
    pub fn in_directory(root: PathBuf) -> Self {
        Self {
            root: Some(root),
            memory: Mutex::new(HashMap::new()),
        }
    }

    /// Records owners in the service's own directory under a state root, which the state store
    /// never reads as a record.
    #[must_use]
    pub fn under_state_root(state_root: &std::path::Path) -> Self {
        Self::in_directory(
            state_root
                .join(soma_local::SERVICE_DIRECTORY)
                .join(DIRECTORY),
        )
    }

    /// Records owners in this process only, for a facade with no state root.
    #[must_use]
    pub fn in_memory() -> Self {
        Self {
            root: None,
            memory: Mutex::new(HashMap::new()),
        }
    }

    /// The tenant recorded for one identity.
    ///
    /// # Errors
    ///
    /// Returns the error of a record that exists but cannot be read.
    pub fn owner(&self, resource: Resource, identity: &str) -> io::Result<Option<String>> {
        let Some(key) = key(identity) else {
            return Ok(None);
        };
        let Some(root) = &self.root else {
            return Ok(self.records().get(&(resource, key)).cloned());
        };
        match fs::read_to_string(root.join(resource.directory()).join(&key)) {
            Ok(tenant) => Ok(Some(tenant)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// What `tenant` may see of one identity.
    ///
    /// # Errors
    ///
    /// Returns the error of a record that exists but cannot be read.
    pub fn access(&self, resource: Resource, identity: &str, tenant: &str) -> io::Result<Access> {
        Ok(match self.owner(resource, identity)? {
            None => Access::Unclaimed,
            Some(owner) if owner == tenant => Access::Owner,
            Some(_) => Access::Other,
        })
    }

    /// Records `tenant` as the owner of an identity no other tenant owns.
    ///
    /// An identity this service would not accept is never recorded; the request that named it is
    /// refused further on.
    ///
    /// # Errors
    ///
    /// Returns the error of a record that could not be written or read back.
    pub fn claim(&self, resource: Resource, identity: &str, tenant: &str) -> io::Result<Claim> {
        let Some(key) = key(identity) else {
            return Ok(Claim::Kept);
        };
        let Some(root) = &self.root else {
            let mut records = self.records();
            return Ok(match records.get(&(resource, key.clone())) {
                Some(owner) if owner == tenant => Claim::Kept,
                Some(_) => Claim::Refused,
                None => {
                    records.insert((resource, key), tenant.to_owned());
                    Claim::New
                }
            });
        };
        let directory = root.join(resource.directory());
        fs::create_dir_all(&directory)?;
        let staged = directory.join(format!(".{key}.{}", unique()));
        let written = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
            .and_then(|mut file| {
                file.write_all(tenant.as_bytes())?;
                file.sync_all()
            })
            .and_then(|()| fs::hard_link(&staged, directory.join(&key)));
        let _ignored = fs::remove_file(&staged);
        match written {
            Ok(()) => {
                sync_directory(&directory)?;
                Ok(Claim::New)
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Ok(match self.access(resource, identity, tenant)? {
                    Access::Owner => Claim::Kept,
                    Access::Other | Access::Unclaimed => Claim::Refused,
                })
            }
            Err(error) => Err(error),
        }
    }

    /// Removes the record of one identity, so it may be taken again.
    ///
    /// # Errors
    ///
    /// Returns the error of a record that exists but could not be removed.
    pub fn forget(&self, resource: Resource, identity: &str) -> io::Result<()> {
        let Some(key) = key(identity) else {
            return Ok(());
        };
        let Some(root) = &self.root else {
            self.records().remove(&(resource, key));
            return Ok(());
        };
        match fs::remove_file(root.join(resource.directory()).join(key)) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }

    fn records(&self) -> std::sync::MutexGuard<'_, HashMap<(Resource, String), String>> {
        self.memory.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Makes the new name durable. Windows cannot open a directory as a file, and commits a link
/// with the file it names, so there is nothing more to do there.
fn sync_directory(directory: &std::path::Path) -> io::Result<()> {
    if cfg!(unix) {
        fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

/// The record name of one identity: its lowercase hexadecimal, or none for anything else, so a
/// caller's string never becomes a path.
fn key(identity: &str) -> Option<String> {
    (!identity.is_empty()
        && identity.len() <= MAX_IDENTITY_BYTES
        && identity.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| identity.to_ascii_lowercase())
}

/// A name no other staged record of this process, or of another, carries at the same moment.
fn unique() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!(
        "{}.{}.{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests;
