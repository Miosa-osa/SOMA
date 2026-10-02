//! Forwarding a request about another host's sandbox to the runner that owns it (contract C2).
//!
//! A plain HTTP client may send everything to `run-us.miosa.ai`; the runner that receives a call
//! for a sandbox it does not hold passes it, unchanged, to the owner over the private network
//! and returns the owner's answer. The owner authenticates the forwarded request against its
//! own key table, exactly as if the client had called it directly, so forwarding grants
//! nothing: the forwarding runner is only a pipe. A request that arrived forwarded is never
//! forwarded again.

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http2::SendRequest;
use hyper_util::rt::TokioIo;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

use crate::runner::{
    config::{ControlPlaneConfig, PeersConfig},
    control_plane::Client,
    limits::{Activity, IdleIo, TLS_HANDSHAKE_TIMEOUT},
    service::{Runner, RunnerRequest, RunnerResponse},
    transport::serve_http,
};

/// How long reaching a peer may take before it counts as unreachable.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// A forwarded body is relayed in chunks through a buffer this deep.
const RELAY_BUFFER: usize = 16;
/// The response headers a forwarded answer keeps.
const RELAYED_HEADERS: [&str; 5] = [
    "content-type",
    "retry-after",
    "server-timing",
    "soma-runner-url",
    "x-miosa-soma-server-tti-us",
];

/// The other runners of the region, by host tag.
pub struct Peers {
    peers: HashMap<char, Peer>,
}

struct Peer {
    client: Client,
    link: tokio::sync::Mutex<Option<SendRequest<Full<Bytes>>>>,
}

impl Peers {
    /// Builds a client for every configured peer; no connection is opened yet.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error for an unusable certificate, key, or peer address.
    pub fn new(config: &PeersConfig) -> io::Result<Self> {
        let mut peers = HashMap::new();
        for (tag, peer) in &config.runners {
            let link = ControlPlaneConfig {
                url: format!("https://{}", peer.address),
                server_name: peer.server_name.clone(),
                ca: config.ca.clone(),
                certificate: config.certificate.clone(),
                private_key: config.private_key.clone(),
            };
            peers.insert(
                *tag,
                Peer {
                    client: Client::with_alpn(&link, b"h2")?,
                    link: tokio::sync::Mutex::new(None),
                },
            );
        }
        Ok(Self { peers })
    }

    /// Passes `request` to the runner holding `tag` and returns its answer, or `None` when no
    /// such runner is configured or it cannot be reached.
    pub async fn forward(&self, tag: char, request: &RunnerRequest) -> Option<RunnerResponse> {
        let peer = self.peers.get(&tag)?;
        let mut sender = peer.sender().await?;
        let mut outgoing = http::Request::builder()
            .method(request.method.clone())
            .uri(request.path.as_str())
            .header(http::header::CONTENT_TYPE, "application/json");
        if let Some(authorization) = &request.authorization {
            outgoing = outgoing.header(http::header::AUTHORIZATION, authorization);
        }
        let outgoing = outgoing.body(Full::new(request.body.clone())).ok()?;
        let Ok(response) = sender.send_request(outgoing).await else {
            // The link is gone; the next request reconnects.
            peer.link.lock().await.take();
            return None;
        };
        let status = response.status().as_u16();
        let headers = RELAYED_HEADERS
            .iter()
            .filter_map(|name| {
                let value = response.headers().get(*name)?.to_str().ok()?;
                Some((*name, value.to_owned()))
            })
            .collect();
        let (relay, stream) = tokio::sync::mpsc::channel(RELAY_BUFFER);
        let mut body = response.into_body();
        tokio::spawn(async move {
            while let Some(Ok(frame)) = body.frame().await {
                if let Ok(data) = frame.into_data()
                    && relay.send(data).await.is_err()
                {
                    break;
                }
            }
        });
        Some(RunnerResponse {
            status,
            headers,
            body: Vec::new(),
            stream: Some(stream),
            journal: None,
        })
    }
}

impl Peer {
    /// A live link to the peer, opened on first use and reopened after it fails.
    async fn sender(&self) -> Option<SendRequest<Full<Bytes>>> {
        let mut link = self.link.lock().await;
        if let Some(sender) = link.as_ref()
            && !sender.is_closed()
        {
            return Some(sender.clone());
        }
        let sender = tokio::time::timeout(CONNECT_TIMEOUT, self.client.connect_h2())
            .await
            .ok()?
            .ok()?;
        *link = Some(sender.clone());
        Some(sender)
    }
}

/// Binds the private listener on which peers forward requests to this runner.
///
/// # Errors
///
/// Returns the bind failure or an unusable certificate, key, or CA.
pub async fn bind_private(
    config: &PeersConfig,
) -> io::Result<(tokio::net::TcpListener, tokio_rustls::TlsAcceptor)> {
    let invalid = |error: &dyn std::fmt::Display| {
        io::Error::new(io::ErrorKind::InvalidData, format!("peers TLS: {error}"))
    };
    let mut roots = rustls::RootCertStore::empty();
    for certificate in CertificateDer::pem_file_iter(&config.ca).map_err(|error| invalid(&error))? {
        roots
            .add(certificate.map_err(|error| invalid(&error))?)
            .map_err(|error| invalid(&error))?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::clone(&provider),
    )
    .build()
    .map_err(|error| invalid(&error))?;
    let chain = CertificateDer::pem_file_iter(&config.certificate)
        .map_err(|error| invalid(&error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| invalid(&error))?;
    let key = PrivateKeyDer::from_pem_file(&config.private_key).map_err(|error| invalid(&error))?;
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| invalid(&error))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)
        .map_err(|error| invalid(&error))?;
    tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    Ok((listener, tokio_rustls::TlsAcceptor::from(Arc::new(tls))))
}

/// Serves forwarded requests from peers until the listener fails.
pub async fn serve_private(
    listener: tokio::net::TcpListener,
    tls: tokio_rustls::TlsAcceptor,
    runner: Arc<Runner>,
) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let (tls, runner) = (tls.clone(), Arc::clone(&runner));
        tokio::spawn(async move {
            let Ok(Ok(stream)) =
                tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, tls.accept(stream)).await
            else {
                return;
            };
            let activity = Activity::new();
            serve_http(
                TokioIo::new(IdleIo::new(stream, &activity)),
                &activity,
                runner,
                true,
            )
            .await;
        });
    }
}
