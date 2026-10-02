//! The public SOMA runner: this host answering internet clients directly.
//!
//! The runner is the one-hop path of `ONE-HOP-RUNNER-DESIGN-2026-10-02.md`: TLS and QUIC end
//! here, the API key is checked against a table held in memory, and the sandbox is created,
//! commanded, or destroyed through the same facade the loopback service drives. Nothing on the
//! request path leaves this process. The control plane feeds the key table ahead of time and
//! receives the paperwork afterwards, through the journal and its shipper.
//!
//! It is off unless the service is started with a runner configuration file, and it never
//! replaces the loopback listener: that one stays for local and administrative use.

pub mod backend;
pub mod clock;
pub mod config;
pub mod control_plane;
pub mod feed;
pub mod feed_client;
pub mod idle;
pub mod ids;
pub mod journal;
pub mod journal_reader;
pub mod keys;
pub mod limits;
pub mod owner;
pub mod peers;
pub mod principal;
pub mod public_wire;
pub mod quic;
pub mod rate_limit;
pub mod refusals;
pub mod sandboxes;
pub mod service;
pub mod shipper;
pub mod tls;
pub mod transport;

use std::{io, net::SocketAddr, sync::Arc, thread, time::Duration};

pub use backend::{Backend, FacadeOpener};
pub use config::RunnerConfig;
pub use journal::Journal;
pub use keys::KeyTable;
pub use service::Runner;

/// A started runner.
pub struct RunnerHandle {
    /// The bound public address; TCP and UDP share it.
    pub address: SocketAddr,
    pub thread: thread::JoinHandle<()>,
}

/// How often the runner looks for expired sandboxes and stale destroyed records.
const REAP_INTERVAL: Duration = Duration::from_secs(1);

/// Starts the runner on its own asynchronous runtime, on its own thread.
///
/// Everything that can fail on bad configuration fails here, before this returns: the
/// certificate is read, the journal is opened, and both public sockets are bound. A runner that
/// starts is therefore a runner that can serve, and a misconfiguration stops the service at
/// startup rather than leaving it half listening.
///
/// # Errors
///
/// Returns the first setup failure: an unreadable certificate or key, a journal that cannot be
/// opened, or a public address that cannot be bound.
pub fn spawn(config: RunnerConfig, opener: FacadeOpener) -> io::Result<RunnerHandle> {
    let worker_threads = thread::available_parallelism().map_or(4, |count| count.get().min(16));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .thread_name("soma-runner")
        .enable_all()
        .build()?;
    let config = Arc::new(config);
    let started = runtime.block_on(start(Arc::clone(&config), opener))?;
    let address = started.address;
    let thread = thread::Builder::new()
        .name("soma-runner".to_owned())
        .spawn(move || {
            runtime.block_on(started.serve());
        })?;
    Ok(RunnerHandle { address, thread })
}

/// The bound, ready-to-serve runner and everything it runs beside its listeners.
struct Started {
    runner: Arc<Runner>,
    /// The private listener other runners forward to, when peers are configured.
    private: Option<(tokio::net::TcpListener, tokio_rustls::TlsAcceptor)>,
    address: SocketAddr,
    tcp: tokio::net::TcpListener,
    quic: quinn::Endpoint,
    tls: Arc<tls::CertificateStore>,
    config: Arc<RunnerConfig>,
}

async fn start(config: Arc<RunnerConfig>, opener: FacadeOpener) -> io::Result<Started> {
    let certificates = Arc::new(tls::CertificateStore::load(
        config.tls.certificate.clone(),
        config.tls.private_key.clone(),
    )?);
    let control_plane = Arc::new(control_plane::Client::new(&config.control_plane)?);
    let journal = Journal::open(&config.journal.directory, &config.runner)?;
    let backend = Backend::new(opener);
    let mut runner = Runner::new(Arc::clone(&config), backend, journal);
    let private = match &config.peers {
        Some(peers) => {
            runner.set_peers(peers::Peers::new(peers)?);
            Some(peers::bind_private(peers).await?)
        }
        None => None,
    };
    let runner = Arc::new(runner);
    runner.recover_sandboxes().await;
    let tcp = tokio::net::TcpListener::bind(config.listen).await?;
    // HTTP/3 binds the port TCP got, so an ephemeral port in tests is one port for both.
    let address = tcp.local_addr()?;
    let quic = quic::bind_quic(address, &certificates)?;
    feed_client::spawn(Arc::clone(&control_plane), Arc::clone(runner.keys()));
    shipper::spawn(
        control_plane,
        runner.journal().clone(),
        config.journal.batch_lines,
    );
    eprintln!("soma-api: runner {} listening on {address}", config.runner);
    Ok(Started {
        runner,
        private,
        address,
        tcp,
        quic,
        tls: certificates,
        config,
    })
}

impl Started {
    async fn serve(self) {
        let reaper = Arc::clone(&self.runner);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(REAP_INTERVAL);
            loop {
                interval.tick().await;
                reaper.reap().await;
            }
        });
        tokio::spawn(tls::watch(Arc::clone(&self.tls)));
        if let Some((listener, acceptor)) = self.private {
            tokio::spawn(peers::serve_private(
                listener,
                acceptor,
                Arc::clone(&self.runner),
            ));
        }
        let per_ip = limits::PerIp::new(self.config.max_connections_per_ip);
        let tcp = transport::serve_tcp(
            self.tcp,
            Arc::clone(&self.tls),
            Arc::clone(&self.runner),
            self.config.max_connections,
            Arc::clone(&per_ip),
        );
        let quic = quic::serve_quic(self.quic, Arc::clone(&self.runner), per_ip);
        tokio::join!(tcp, quic);
    }
}
