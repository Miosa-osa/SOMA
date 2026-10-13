//! The per-volume data key of ADR 0053 Step A: generated at create, sealed under the host key,
//! destroyed with the volume.
//!
//! The key is what proves a destruction. The volume's bytes live on this host's encrypted data
//! mount, and the data key is ADR 0034's rule applied to a volume: a sealed, per-volume secret
//! whose removal is the erasure, because a host cannot claim more about the medium than that.
//!
//! The master key is a root-only file on the encrypted mount and never leaves it: it is created
//! `0600` on first use, and a host where it cannot be created or read refuses the volume rather
//! than writing a key down somewhere else.
//!
//! The sealed key sits beside the volume's write-ahead records, under the durable record root and
//! never in the volume directory, so destroying a volume has two things to remove rather than one.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead as _, KeyInit as _},
};
use zeroize::Zeroize as _;

use crate::VolumeId;

mod error;
pub use error::KeyError;

/// The environment variable naming the host's master key file.
pub const KEY_FILE_ENV: &str = "SOMA_HOSTD_VOLUME_KEY_FILE";
/// The master key file the reference Step A host carries, on its encrypted data mount.
pub const DEFAULT_KEY_FILE: &str = "/srv/miosa-enc/soma/volume-master.key";
/// The master key's length, in bytes.
pub const KEY_LEN: usize = 32;
/// The nonce length of the cipher this seals with, in bytes.
pub const NONCE_LEN: usize = 12;
/// The sealed data key's length: the key plus its authentication tag.
pub const SEALED_LEN: usize = KEY_LEN + 16;
/// The subtree of the record root holding one sealed key per volume.
pub const KEYS_DIR: &str = "keys";
/// The file one volume's sealed key is written to, under its own key directory.
pub const SEALED_FILE: &str = "dek";

/// One volume's data key, sealed and identified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SealedKey {
    /// The identity the sidecar carries, derived from the sealed bytes.
    pub dek_id: [u8; 16],
    /// The nonce the sealed key was produced with, which belongs with the ciphertext.
    pub nonce: [u8; NONCE_LEN],
    /// The sealed key itself. Never written to a volume directory and never sent anywhere.
    pub sealed: [u8; SEALED_LEN],
}

/// The per-volume key operations a store performs.
///
/// It is a seam because the real one reads a root-only file on a host's encrypted mount, while a
/// test needs a deterministic answer and no key material on disk.
pub trait VolumeKeys {
    /// Generates a volume's data key and seals it under the host key.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the host key cannot be read or created, or when the operating
    /// system refuses to produce random bytes.
    fn seal(&self, volume: VolumeId) -> Result<SealedKey, KeyError>;

    /// Writes one sealed key where a later start can unseal it.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the record directory or the file cannot be written.
    fn record(&self, volume: VolumeId, key: &SealedKey) -> Result<(), KeyError>;

    /// Reads back one volume's sealed key.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::Missing`] when no sealed key exists for this volume.
    fn read(&self, volume: VolumeId) -> Result<SealedKey, KeyError>;

    /// Unseals one data key under the host key.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::Seal`] when the frame is not the one this host sealed.
    fn unseal(&self, key: &SealedKey) -> Result<[u8; KEY_LEN], KeyError>;

    /// Destroys one volume's sealed key, which is how its destruction is proven.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the sealed key exists and cannot be removed.
    fn destroy(&self, volume: VolumeId) -> Result<(), KeyError>;
}

/// The host's own keyring: a master key file on the encrypted mount, and one sealed key per volume
/// under the durable record root.
pub struct HostKeyring {
    master: PathBuf,
    records: PathBuf,
}

impl HostKeyring {
    /// The keyring this host is configured with.
    #[must_use]
    pub fn from_env(records: &Path) -> Self {
        let master = std::env::var(KEY_FILE_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map_or_else(|| PathBuf::from(DEFAULT_KEY_FILE), PathBuf::from);
        Self::at(master, records)
    }

    /// The keyring reading `master` and recording under `records`.
    #[must_use]
    pub fn at(master: impl Into<PathBuf>, records: &Path) -> Self {
        Self {
            master: master.into(),
            records: records.to_path_buf(),
        }
    }

    /// The file one volume's sealed key is recorded in.
    ///
    /// It is a subtree of the record root rather than the volume's own record directory: the
    /// write-ahead records are one append-only family whose names are sequence numbers, and a
    /// key file among them would be counted as a record by every reader of that family.
    #[must_use]
    pub fn sealed_path(&self, volume: VolumeId) -> PathBuf {
        self.records
            .join(KEYS_DIR)
            .join(volume.hex())
            .join(SEALED_FILE)
    }

    /// Reads the master key, creating it `0600` when this host does not have one yet.
    fn master_key(&self) -> Result<[u8; KEY_LEN], KeyError> {
        match read_master(&self.master) {
            Ok(key) => Ok(key),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_master(&self.master)
            }
            Err(_) => Err(KeyError::KeyFile("the host key could not be read")),
        }
    }
}

/// Reads a master key file and refuses anything that is not exactly key-sized.
fn read_master(path: &Path) -> std::io::Result<[u8; KEY_LEN]> {
    let mut file = OpenOptions::new().read(true).open(path)?;
    let mut key = [0_u8; KEY_LEN];
    file.read_exact(&mut key)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the master key file is longer than a key",
        ));
    }
    Ok(key)
}

/// Creates the master key on first use, root-only, and never replaces one that exists.
fn create_master(path: &Path) -> Result<[u8; KEY_LEN], KeyError> {
    let Some(parent) = path.parent() else {
        return Err(KeyError::KeyFile("the host key has no directory"));
    };
    fs::create_dir_all(parent).map_err(|_| KeyError::KeyFile("the host key directory"))?;
    let mut key = [0_u8; KEY_LEN];
    random(&mut key)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| KeyError::KeyFile("the host key could not be created"))?;
    file.write_all(&key)
        .map_err(|_| KeyError::KeyFile("the host key could not be written"))?;
    file.sync_all()
        .map_err(|_| KeyError::KeyFile("the host key could not be synced"))?;
    Ok(key)
}

/// Fills `bytes` from the operating system.
fn random(bytes: &mut [u8]) -> Result<(), KeyError> {
    getrandom::getrandom(bytes).map_err(|_| KeyError::Random)
}

/// The identity of one sealed key: a digest of the sealed bytes, so two volumes sealed under the
/// same master key are still distinguishable and a shortened frame cannot pass as another.
fn dek_id(sealed: &[u8]) -> [u8; 16] {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(sealed);
    let mut id = [0_u8; 16];
    id.copy_from_slice(&digest[..16]);
    id
}

impl VolumeKeys for HostKeyring {
    fn seal(&self, _volume: VolumeId) -> Result<SealedKey, KeyError> {
        let master = self.master_key()?;
        let mut dek = [0_u8; KEY_LEN];
        random(&mut dek)?;
        let mut nonce = [0_u8; NONCE_LEN];
        random(&mut nonce)?;
        let cipher = ChaCha20Poly1305::new((&master).into());
        let sealed = cipher
            .encrypt(Nonce::from_slice(&nonce), dek.as_slice())
            .map_err(|_| KeyError::Seal)?;
        if sealed.len() != SEALED_LEN {
            return Err(KeyError::Seal);
        }
        let mut bytes = [0_u8; SEALED_LEN];
        bytes.copy_from_slice(&sealed);
        let id = dek_id(&bytes);
        dek.zeroize();
        Ok(SealedKey {
            dek_id: id,
            nonce,
            sealed: bytes,
        })
    }

    fn record(&self, volume: VolumeId, key: &SealedKey) -> Result<(), KeyError> {
        let path = self.sealed_path(volume);
        let Some(parent) = path.parent() else {
            return Err(KeyError::Record("the sealed key has no directory"));
        };
        fs::create_dir_all(parent).map_err(|_| KeyError::Record("sealed key directory"))?;
        let mut body = Vec::with_capacity(NONCE_LEN + SEALED_LEN);
        body.extend_from_slice(&key.nonce);
        body.extend_from_slice(&key.sealed);
        // The file is replaced whole: a sealed key is one fixed-size frame, and writing over it
        // is how a re-record after a lost answer stays a single key rather than two.
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| KeyError::Record("sealed key file"))?;
        file.write_all(&body)
            .map_err(|_| KeyError::Record("sealed key write"))?;
        file.sync_all()
            .map_err(|_| KeyError::Record("sealed key sync"))
    }

    fn read(&self, volume: VolumeId) -> Result<SealedKey, KeyError> {
        let path = self.sealed_path(volume);
        let body = fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                KeyError::Missing
            } else {
                KeyError::Record("sealed key read")
            }
        })?;
        if body.len() != NONCE_LEN + SEALED_LEN {
            return Err(KeyError::Record("the sealed key file is not one"));
        }
        let mut nonce = [0_u8; NONCE_LEN];
        nonce.copy_from_slice(&body[..NONCE_LEN]);
        let mut sealed = [0_u8; SEALED_LEN];
        sealed.copy_from_slice(&body[NONCE_LEN..]);
        Ok(SealedKey {
            dek_id: dek_id(&sealed),
            nonce,
            sealed,
        })
    }

    fn unseal(&self, key: &SealedKey) -> Result<[u8; KEY_LEN], KeyError> {
        let master = self.master_key()?;
        let cipher = ChaCha20Poly1305::new((&master).into());
        let opened = cipher
            .decrypt(Nonce::from_slice(&key.nonce), key.sealed.as_slice())
            .map_err(|_| KeyError::Seal)?;
        if opened.len() != KEY_LEN {
            return Err(KeyError::Seal);
        }
        let mut dek = [0_u8; KEY_LEN];
        dek.copy_from_slice(&opened);
        Ok(dek)
    }

    fn destroy(&self, volume: VolumeId) -> Result<(), KeyError> {
        match fs::remove_file(self.sealed_path(volume)) {
            Ok(()) => Ok(()),
            // A key that is already gone is the state destroy wanted, so a replay after a lost
            // answer succeeds rather than reporting a volume whose key nobody holds.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(KeyError::Record("sealed key unlink")),
        }
    }
}

#[cfg(test)]
mod tests;
