use std::{io, sync::Arc};

use bytes::Bytes;
use http_body_util::Full;
use hyper::client::conn::{http1::SendRequest, http2};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use rustls::RootCertStore;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use tokio_rustls::TlsConnector;

use crate::runner::config::{ControlPlaneConfig, control_plane_authority};

/// How often an idle runner-to-runner link proves it is alive, and how long it may take.
const PEER_KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(10);

/// The runner's mTLS client to the control plane's private endpoints.
///
/// Both the key feed and the journal shipper use it. Each opens its own HTTP/1.1 connection and
/// keeps it: the feed holds one response open for its whole life, and the shipper's posts are
/// sequential, so neither gains anything from multiplexing.
pub struct Client {
    connector: TlsConnector,
    host: String,
    port: u16,
    server_name: ServerName<'static>,
    authority: String,
}

impl Client {
    /// Loads the fleet CA and this host's client certificate.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error for an unreadable or unusable certificate, key, or URL.
    pub fn new(config: &ControlPlaneConfig) -> io::Result<Self> {
        Self::with_alpn(config, b"http/1.1")
    }

    /// The same client offering one application protocol, `h2` for runner-to-runner links.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error for an unreadable or unusable certificate, key, or URL.
    pub fn with_alpn(config: &ControlPlaneConfig, alpn: &[u8]) -> io::Result<Self> {
        let (host, port) = control_plane_authority(&config.url)?;
        let mut roots = RootCertStore::empty();
        for certificate in
            CertificateDer::pem_file_iter(&config.ca).map_err(|error| pem_error(&error))?
        {
            roots
                .add(certificate.map_err(|error| pem_error(&error))?)
                .map_err(|error| tls_error(&error))?;
        }
        let chain = CertificateDer::pem_file_iter(&config.certificate)
            .map_err(|error| pem_error(&error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| pem_error(&error))?;
        let key =
            PrivateKeyDer::from_pem_file(&config.private_key).map_err(|error| pem_error(&error))?;
        let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|error| tls_error(&error))?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .map_err(|error| tls_error(&error))?;
        tls.alpn_protocols = vec![alpn.to_vec()];
        let name = config.server_name.clone().unwrap_or_else(|| host.clone());
        let server_name = ServerName::try_from(name)
            .map_err(|_| invalid("control_plane.server_name is not a valid TLS name"))?;
        let authority = if port == 443 {
            host.clone()
        } else {
            format!("{host}:{port}")
        };
        Ok(Self {
            connector: TlsConnector::from(Arc::new(tls)),
            host,
            port,
            server_name,
            authority,
        })
    }

    /// Opens one mTLS HTTP/1.1 connection and returns its request handle.
    ///
    /// # Errors
    ///
    /// Returns the connect, TLS, or HTTP handshake failure.
    pub async fn connect(&self) -> io::Result<SendRequest<Full<Bytes>>> {
        let tcp = tokio::net::TcpStream::connect((self.host.as_str(), self.port)).await?;
        tcp.set_nodelay(true)?;
        let tls = self
            .connector
            .connect(self.server_name.clone(), tcp)
            .await?;
        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
            .await
            .map_err(io::Error::other)?;
        tokio::spawn(async move {
            // The connection ends when either side closes it; the request handle then reports
            // the failure to whichever task was using it, and that task reconnects.
            let _ended = connection.await;
        });
        Ok(sender)
    }

    /// Opens one mTLS HTTP/2 connection, multiplexed by every request that clones its handle.
    ///
    /// # Errors
    ///
    /// Returns the connect, TLS, or HTTP/2 handshake failure.
    pub async fn connect_h2(&self) -> io::Result<http2::SendRequest<Full<Bytes>>> {
        let tcp = tokio::net::TcpStream::connect((self.host.as_str(), self.port)).await?;
        tcp.set_nodelay(true)?;
        let tls = self
            .connector
            .connect(self.server_name.clone(), tcp)
            .await?;
        let (sender, connection) = http2::Builder::new(TokioExecutor::new())
            .timer(TokioTimer::new())
            .keep_alive_interval(Some(PEER_KEEPALIVE))
            .keep_alive_timeout(PEER_KEEPALIVE)
            .handshake(TokioIo::new(tls))
            .await
            .map_err(io::Error::other)?;
        tokio::spawn(async move {
            let _ended = connection.await;
        });
        Ok(sender)
    }

    /// Builds a request for `path` on the control plane.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error when `path` cannot form a request URI.
    pub fn request(
        &self,
        method: http::Method,
        path: &str,
        content_type: Option<&'static str>,
        body: Bytes,
    ) -> io::Result<http::Request<Full<Bytes>>> {
        let mut builder = http::Request::builder()
            .method(method)
            .uri(path)
            .header(http::header::HOST, &self.authority);
        if let Some(content_type) = content_type {
            builder = builder.header(http::header::CONTENT_TYPE, content_type);
        }
        builder
            .body(Full::new(body))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
    }
}

fn pem_error(error: &rustls_pki_types::pem::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("PEM: {error}"))
}

fn tls_error(error: &rustls::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("TLS: {error}"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
