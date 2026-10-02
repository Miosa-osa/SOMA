use std::{
    fmt, io,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime},
};

use rustls::{
    server::{ClientHello, ResolvesServerCert},
    sign::CertifiedKey,
};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

/// How often the certificate files are checked for a renewal.
const WATCH_INTERVAL: Duration = Duration::from_secs(5);

/// The public certificate, served to both TCP and QUIC handshakes and reloaded on change.
///
/// The files are re-read when their modification time or size changes. A renewal that cannot be
/// parsed (a half-written file, a key that does not match) is logged and the previous
/// certificate stays in service, so a bad push never takes the listener down.
pub struct CertificateStore {
    certificate: PathBuf,
    private_key: PathBuf,
    current: RwLock<Loaded>,
}

struct Loaded {
    key: Arc<CertifiedKey>,
    stamp: Stamp,
}

type Stamp = [(Option<SystemTime>, u64); 2];

impl CertificateStore {
    /// Loads the certificate chain and private key.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error when either file is unreadable or they do not form a
    /// usable certified key.
    pub fn load(certificate: PathBuf, private_key: PathBuf) -> io::Result<Self> {
        let stamp = stamp(&certificate, &private_key);
        let key = Arc::new(certified_key(&certificate, &private_key)?);
        Ok(Self {
            certificate,
            private_key,
            current: RwLock::new(Loaded { key, stamp }),
        })
    }

    /// Re-reads the files if they changed since the last load.
    ///
    /// Returns whether a new certificate was installed.
    ///
    /// # Errors
    ///
    /// Returns the load failure of a changed pair; the previous certificate stays in service.
    pub fn reload_if_changed(&self) -> io::Result<bool> {
        let stamp = stamp(&self.certificate, &self.private_key);
        if stamp == self.read().stamp {
            return Ok(false);
        }
        let loaded = certified_key(&self.certificate, &self.private_key);
        let mut current = self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The stamp is taken either way, so a broken file is reported once, not every interval.
        current.stamp = stamp;
        current.key = Arc::new(loaded?);
        Ok(true)
    }

    /// The certified key in service now.
    #[must_use]
    pub fn current(&self) -> Arc<CertifiedKey> {
        Arc::clone(&self.read().key)
    }

    /// A TLS server configuration that resolves through this store.
    ///
    /// # Errors
    ///
    /// Returns the protocol-version failure of the crypto provider, which only a broken build
    /// produces.
    pub fn server_config(
        self: &Arc<Self>,
        alpn: &[&[u8]],
        tls13_only: bool,
    ) -> io::Result<rustls::ServerConfig> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = rustls::ServerConfig::builder_with_provider(provider);
        let builder = if tls13_only {
            builder.with_protocol_versions(&[&rustls::version::TLS13])
        } else {
            builder.with_safe_default_protocol_versions()
        }
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut config = builder
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(Resolver(Arc::clone(self))));
        config.alpn_protocols = alpn.iter().map(|protocol| protocol.to_vec()).collect();
        Ok(config)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Loaded> {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Checks for a renewed certificate for the life of the process.
pub async fn watch(store: Arc<CertificateStore>) {
    let mut interval = tokio::time::interval(WATCH_INTERVAL);
    loop {
        interval.tick().await;
        let watched = Arc::clone(&store);
        match tokio::task::spawn_blocking(move || watched.reload_if_changed()).await {
            Ok(Ok(true)) => eprintln!("soma-api: runner certificate reloaded"),
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                eprintln!(
                    "soma-api: runner certificate reload failed, keeping the old one: {error}"
                );
            }
            Err(error) => eprintln!("soma-api: runner certificate watch failed: {error}"),
        }
    }
}

struct Resolver(Arc<CertificateStore>);

impl fmt::Debug for Resolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CertificateStore")
    }
}

impl ResolvesServerCert for Resolver {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.current())
    }
}

fn certified_key(certificate: &Path, private_key: &Path) -> io::Result<CertifiedKey> {
    let chain = CertificateDer::pem_file_iter(certificate)
        .map_err(|error| pem_error(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| pem_error(&error))?;
    if chain.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the certificate file holds no certificate",
        ));
    }
    let key = PrivateKeyDer::from_pem_file(private_key).map_err(|error| pem_error(&error))?;
    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let certified = CertifiedKey::new(chain, signing_key);
    certified
        .keys_match()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(certified)
}

fn stamp(certificate: &Path, private_key: &Path) -> Stamp {
    let one = |path: &Path| {
        std::fs::metadata(path).map_or((None, 0), |metadata| {
            (metadata.modified().ok(), metadata.len())
        })
    };
    [one(certificate), one(private_key)]
}

fn pem_error(error: &rustls_pki_types::pem::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("PEM: {error}"))
}
