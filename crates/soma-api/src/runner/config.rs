use std::{io, net::SocketAddr, path::PathBuf, thread, time::Duration};

use serde::Deserialize;
use soma::MachineShape;

/// The per-key request budget when the feed does not set one, matching the fast-lane edge's own
/// default so a key sees the same ceiling on either path.
pub const DEFAULT_RATE_PER_SECOND: u32 = 300;

/// How old the key feed may grow before new creates are refused.
///
/// Past this the table may still hold a key that was revoked while the feed was down, so the
/// runner stops creating sandboxes for anyone. Existing sandboxes keep being served.
pub const DEFAULT_FEED_STALE_AFTER: Duration = Duration::from_mins(15);

const DEFAULT_TIMEOUT_SECONDS: u64 = 3_600;
const DEFAULT_MAX_CONNECTIONS: usize = 16_384;
const DEFAULT_BATCH_LINES: usize = 500;

/// Everything the public runner needs, read from one JSON file.
///
/// A file rather than a dozen command-line options: the runner is configured by deployment, the
/// file is what an operator diffs between hosts, and unknown fields are refused so a misspelled
/// option is an error at startup instead of a silently ignored setting.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunnerConfig {
    /// This runner's name in the journal, for example `miosa-host-03`.
    pub runner: String,
    /// The single lowercase hex digit every sandbox id minted here starts with.
    pub host_tag: char,
    /// The public address, which must be this host's secondary IP, port 443.
    pub listen: SocketAddr,
    /// The regional runner domain; a misdirected request is pointed at `<tag>.<domain>`.
    pub public_domain: String,
    pub tls: TlsFiles,
    pub control_plane: ControlPlaneConfig,
    pub journal: JournalConfig,
    pub launch: LaunchConfig,
    /// The most creates this host runs at once before answering `runtime_busy`.
    #[serde(default)]
    pub admission: Option<usize>,
    /// The per-key budget for keys whose feed record carries none.
    #[serde(default = "default_rate")]
    pub rate_per_second: u32,
    #[serde(default = "default_stale_seconds")]
    pub feed_stale_after_seconds: u64,
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    /// Connections one client IP may hold, TCP and QUIC together.
    #[serde(default = "default_connections_per_ip")]
    pub max_connections_per_ip: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsFiles {
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

/// The control plane's private mTLS endpoint, used by both the feed and the shipper.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneConfig {
    /// `https://host:port`, reached over the private network.
    pub url: String,
    /// The name the control plane's certificate is issued to, when it is not the URL host.
    #[serde(default)]
    pub server_name: Option<String>,
    pub ca: PathBuf,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalConfig {
    pub directory: PathBuf,
    #[serde(default = "default_batch_lines")]
    pub batch_lines: usize,
}

/// What a create launches, and what the compact create answer reports about it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchConfig {
    pub image: String,
    pub shape: MachineShape,
    /// The template id the compact answer reports, as the standard path's `template_id`.
    pub template_id: String,
    #[serde(default = "default_timeout_seconds")]
    pub default_timeout_seconds: u64,
}

impl RunnerConfig {
    /// Reads and validates the runner configuration file.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error naming the first field that is unusable.
    pub fn load(path: &std::path::Path) -> io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::parse(&bytes)
    }

    /// Parses and validates a configuration document.
    ///
    /// # Errors
    ///
    /// Returns an invalid-data error naming the first field that is unusable.
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let config: Self = serde_json::from_slice(bytes).map_err(|error| invalid(&error))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> io::Result<()> {
        if !self.host_tag.is_ascii_hexdigit() || self.host_tag.is_ascii_uppercase() {
            return Err(invalid(&"host_tag must be one lowercase hex digit"));
        }
        if !is_name(&self.runner) {
            return Err(invalid(
                &"runner must be 1 to 64 lowercase alphanumeric or hyphen bytes",
            ));
        }
        if self.public_domain.is_empty() || !self.public_domain.is_ascii() {
            return Err(invalid(&"public_domain must be an ASCII host name"));
        }
        if self.admission == Some(0)
            || self.max_connections == 0
            || self.max_connections_per_ip == 0
            || self.journal.batch_lines == 0
        {
            return Err(invalid(
                &"admission, max_connections and batch_lines must be positive",
            ));
        }
        if !(1..=86_400).contains(&self.launch.default_timeout_seconds) {
            return Err(invalid(&"default_timeout_seconds must be 1 to 86400"));
        }
        soma::OciImage::parse(self.launch.image.clone())
            .map_err(|_| invalid(&"launch.image is not a valid OCI reference"))?;
        control_plane_authority(&self.control_plane.url)?;
        Ok(())
    }

    /// The create admission cap: the configured one, or one per hardware thread.
    #[must_use]
    pub fn admission(&self) -> usize {
        self.admission.unwrap_or_else(|| {
            thread::available_parallelism().map_or(32, std::num::NonZeroUsize::get)
        })
    }

    #[must_use]
    pub const fn feed_stale_after(&self) -> Duration {
        Duration::from_secs(self.feed_stale_after_seconds)
    }
}

/// Splits `https://host:port` into the host and port the client dials.
///
/// # Errors
///
/// Returns an invalid-data error for anything that is not an `https` URL with a host.
pub fn control_plane_authority(url: &str) -> io::Result<(String, u16)> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| invalid(&"control_plane.url must start with https://"))?;
    let authority = rest.trim_end_matches('/');
    if authority.is_empty() || authority.contains('/') {
        return Err(invalid(&"control_plane.url must be https://host[:port]"));
    }
    // The control plane is reached by name or IPv4 address over the private network, so a
    // bracketed IPv6 literal is not accepted rather than half supported.
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (
            host,
            port.parse()
                .map_err(|_| invalid(&"control_plane.url has an invalid port"))?,
        ),
        None => (authority, 443),
    };
    if host.is_empty() || host.contains(['[', ']']) {
        return Err(invalid(&"control_plane.url must name a host"));
    }
    Ok((host.to_owned(), port))
}

fn is_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn invalid(reason: &dyn std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("runner configuration: {reason}"),
    )
}

const fn default_rate() -> u32 {
    DEFAULT_RATE_PER_SECOND
}

const fn default_stale_seconds() -> u64 {
    DEFAULT_FEED_STALE_AFTER.as_secs()
}

const fn default_max_connections() -> usize {
    DEFAULT_MAX_CONNECTIONS
}

const fn default_connections_per_ip() -> usize {
    crate::runner::limits::DEFAULT_CONNECTIONS_PER_IP
}

const fn default_batch_lines() -> usize {
    DEFAULT_BATCH_LINES
}

const fn default_timeout_seconds() -> u64 {
    DEFAULT_TIMEOUT_SECONDS
}

#[cfg(test)]
#[path = "config_tests.rs"]
pub(crate) mod tests;
