//! The suspension store (ADR 0034): sealed Suspensions on the host's encrypted data mount, each
//! under a per-Suspension data key that is itself sealed under the host key.
//!
//! A suspension stages its three plaintext members (`memory.raw`, `overlay.raw`,
//! `state.somasnap`, the snapshot codec's names) in a directory of its own; [`Store::seal`] seals
//! them into `<root>/<id>/`, writes the header last, and removes the plaintext. [`Store::open`]
//! decrypts a Suspension back into a staging directory for a resume, verifying every member
//! against the header, and refuses one that has expired. [`Store::destroy`] removes the members
//! and destroys the data key, which is what a destruction means here: the key is gone.
//!
//! The data keys use the volume keyring's sealing, under a records directory of the store's own,
//! so a Suspension's key and a volume's key never share a name.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

use super::seal::{DataKey, SealError, open_member, seal_member};
use super::{Header, HeaderError, MEMBERS, Member, MemberRecord};
use crate::VolumeId;
use crate::ids::SuspensionId;
use crate::volume::keyring::{HostKeyring, KeyError, VolumeKeys as _};

const HEADER_FILE: &str = "header";

/// The plaintext file a member is staged as, which is the snapshot codec's own name for it.
const fn staged_name(member: Member) -> &'static str {
    match member {
        Member::Memory => "memory.raw",
        Member::Overlay => "overlay.raw",
        Member::State => "state.somasnap",
    }
}

const fn sealed_name(member: Member) -> &'static str {
    match member {
        Member::Memory => "memory.sealed",
        Member::Overlay => "overlay.sealed",
        Member::State => "state.sealed",
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Key(KeyError),
    Seal(SealError),
    Header(HeaderError),
    /// The Suspension expired; it has been destroyed and cannot be resumed.
    Expired,
    /// No Suspension of that id exists here.
    Missing,
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::NotFound {
            Self::Missing
        } else {
            Self::Io(error)
        }
    }
}

impl From<SealError> for StoreError {
    fn from(error: SealError) -> Self {
        Self::Seal(error)
    }
}

/// One host's suspension store.
pub struct Store {
    root: PathBuf,
    keys: HostKeyring,
}

/// The keyring names a sealed key by a 16-byte identity; a Suspension's is its id.
fn key_name(id: SuspensionId) -> Result<VolumeId, StoreError> {
    VolumeId::new(*id.as_bytes()).map_err(|_| StoreError::Header(HeaderError::ZeroId("id")))
}

impl Store {
    /// A store at `root`, sealing data keys under the host key file `master`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>, master: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let keys = HostKeyring::at(master, &root.join("records"));
        Self { root, keys }
    }

    fn directory(&self, id: SuspensionId) -> PathBuf {
        self.root.join(id.hex())
    }

    /// Seals the members staged in `staged` into a new Suspension described by `header` (whose
    /// member records are filled in here), then removes the plaintext.
    ///
    /// # Errors
    ///
    /// Returns the first failure; nothing is published then, the partial directory and the data
    /// key are removed, and the staged plaintext is left for the caller to discard.
    pub fn seal(&self, staged: &Path, mut header: Header) -> Result<Header, StoreError> {
        header.validate().map_err(StoreError::Header)?;
        let id = header.id;
        let partial = self.root.join(format!("{}.partial", id.hex()));
        let result = self.seal_into(staged, &partial, &mut header);
        if let Err(error) = result {
            let _ignored = fs::remove_dir_all(&partial);
            let _ignored = key_name(id).map(|name| self.keys.destroy(name));
            return Err(error);
        }
        fs::rename(&partial, self.directory(id))?;
        for member in MEMBERS {
            let _ignored = fs::remove_file(staged.join(staged_name(member)));
        }
        Ok(header)
    }

    fn seal_into(
        &self,
        staged: &Path,
        partial: &Path,
        header: &mut Header,
    ) -> Result<(), StoreError> {
        fs::create_dir_all(&self.root)?;
        fs::create_dir(partial)?;
        let name = key_name(header.id)?;
        let sealed_key = self.keys.seal(name).map_err(StoreError::Key)?;
        self.keys
            .record(name, &sealed_key)
            .map_err(StoreError::Key)?;
        let key = DataKey::from_bytes(self.keys.unseal(&sealed_key).map_err(StoreError::Key)?);
        for (member, slot) in MEMBERS.iter().zip(&mut header.members) {
            let source = staged.join(staged_name(*member));
            let mut out = BufWriter::new(File::create(partial.join(sealed_name(*member)))?);
            *slot = match File::open(&source) {
                Ok(file) => seal_member(
                    &key,
                    header.id,
                    *member,
                    &mut BufReader::new(file),
                    &mut out,
                )?,
                // A Generation with no writable storage has no overlay member to stage.
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && *member == Member::Overlay =>
                {
                    seal_member(&key, header.id, *member, &mut std::io::empty(), &mut out)?
                }
                Err(error) => return Err(error.into()),
            };
            out.into_inner()
                .map_err(|error| StoreError::Io(error.into_error()))?
                .sync_all()?;
        }
        let bytes = header.encode().map_err(StoreError::Header)?;
        fs::write(partial.join(HEADER_FILE), bytes)?;
        File::open(partial)?.sync_all()?;
        Ok(())
    }

    /// Reads a Suspension's header.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Missing`] for an unknown id and a header error for a damaged one.
    pub fn header(&self, id: SuspensionId) -> Result<Header, StoreError> {
        let bytes = fs::read(self.directory(id).join(HEADER_FILE))?;
        Header::decode(&bytes).map_err(StoreError::Header)
    }

    /// Decrypts a Suspension into `target` for a resume, after checking it has not expired.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Expired`] for an expired Suspension, which is destroyed, and the
    /// first verification failure otherwise; a Suspension that does not verify is destroyed
    /// rather than resumed, and the caller discards whatever `target` holds.
    pub fn open(
        &self,
        id: SuspensionId,
        now_unix: u64,
        target: &Path,
    ) -> Result<Header, StoreError> {
        let header = self.header(id)?;
        if header.expired_at(now_unix) {
            self.destroy(id)?;
            return Err(StoreError::Expired);
        }
        let name = key_name(id)?;
        let sealed_key = self.keys.read(name).map_err(StoreError::Key)?;
        let key = DataKey::from_bytes(self.keys.unseal(&sealed_key).map_err(StoreError::Key)?);
        fs::create_dir_all(target)?;
        let directory = self.directory(id);
        for (member, record) in MEMBERS.iter().zip(&header.members) {
            if let Err(error) = self.open_one(&key, &header, *member, record, &directory, target) {
                if matches!(error, StoreError::Seal(_)) {
                    let _ignored = self.destroy(id);
                }
                return Err(error);
            }
        }
        Ok(header)
    }

    fn open_one(
        &self,
        key: &DataKey,
        header: &Header,
        member: Member,
        record: &MemberRecord,
        directory: &Path,
        target: &Path,
    ) -> Result<(), StoreError> {
        let mut sealed = BufReader::new(File::open(directory.join(sealed_name(member)))?);
        // An empty overlay is the Generation with no writable storage: nothing is staged for it.
        if member == Member::Overlay && record.bytes == 0 {
            return Ok(open_member(
                key,
                header.id,
                member,
                record,
                &mut sealed,
                &mut std::io::sink(),
            )?);
        }
        let mut out = BufWriter::new(File::create(target.join(staged_name(member)))?);
        open_member(key, header.id, member, record, &mut sealed, &mut out)?;
        out.into_inner()
            .map_err(|error| StoreError::Io(error.into_error()))?
            .sync_all()?;
        Ok(())
    }

    /// Removes a Suspension and destroys its data key. Destroying one already gone succeeds.
    ///
    /// # Errors
    ///
    /// Returns the failure to remove the members or the key.
    pub fn destroy(&self, id: SuspensionId) -> Result<(), StoreError> {
        match fs::remove_dir_all(self.directory(id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::Io(error)),
        }
        self.keys.destroy(key_name(id)?).map_err(StoreError::Key)
    }
}
