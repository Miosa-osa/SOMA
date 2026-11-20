//! Guest-port tunnels: what both HTTP faces of this service share.
//!
//! `GET /v1/sandboxes/{id}/tunnel/{port}` (loopback) and
//! `GET /api/v1/sandboxes/{id}/tunnel/{port}` (the public runner) upgrade the connection to
//! raw bytes and pump them to a port of the sandbox's guest. The guest's network lives in a
//! namespace only the network broker can enter, so the stream itself comes from the process
//! holding the sandbox ([`crate::SandboxFacade::tunnel`]); this module owns the rules around it:
//!
//! - which ports may be asked for ([`TunnelPolicy::ports`], enforced here and again by the
//!   broker, so a widened list here cannot reach a port the broker was not configured for),
//! - how many tunnels one sandbox and one process may hold at once ([`TunnelTable`]),
//! - how long an idle tunnel lives,
//! - how each failure is named to the caller ([`Refusal`]),
//! - and the two byte pumps, one for the blocking loopback server and one for the runner's
//!   asynchronous one.
//!
//! The upgrade token is `soma-tunnel`: `Connection: Upgrade` and `Upgrade: soma-tunnel` on a
//! `GET`, answered `101` with the same two headers. After the `101` the connection is the guest
//! port, byte for byte, with half-close honoured in both directions.

use std::{
    collections::HashMap,
    ops::RangeInclusive,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{envelope::ApiError, facade::TunnelFailure};

mod guest;
#[cfg(unix)]
mod pump;

pub use guest::{AsyncGuest, GuestStream, into_async};
#[cfg(not(unix))]
pub use guest::{pump, pump_blocking};
#[cfg(unix)]
pub use pump::{pump, pump_blocking};

/// The `Upgrade` token a tunnel request names and its `101` repeats.
pub const UPGRADE_TOKEN: &str = "soma-tunnel";
/// The guest ports a tunnel may reach when an operator names none: the live browser's control
/// and view ports.
pub const DEFAULT_PORTS: RangeInclusive<u16> = 9223..=9224;
/// Tunnels one sandbox may hold at once.
pub const DEFAULT_PER_SANDBOX: usize = 16;
/// Tunnels one process may hold at once.
pub const DEFAULT_TOTAL: usize = 4_096;
/// How long a tunnel that moves no bytes lives.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(600);
/// The environment variable that names the allowed guest ports for the loopback face:
/// comma-separated ports and `FIRST-LAST` ranges.
pub const PORTS_ENV: &str = "SOMA_API_TUNNEL_PORTS";
/// The environment variable that caps tunnels per sandbox on the loopback face.
pub const PER_SANDBOX_ENV: &str = "SOMA_API_TUNNEL_MAX_PER_SANDBOX";
/// The environment variable that caps tunnels per process on the loopback face.
pub const TOTAL_ENV: &str = "SOMA_API_TUNNEL_MAX_TOTAL";
/// The environment variable that sets the idle timeout, in seconds, on the loopback face.
pub const IDLE_ENV: &str = "SOMA_API_TUNNEL_IDLE_SECONDS";

/// What a tunnel may do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TunnelPolicy {
    pub ports: Vec<RangeInclusive<u16>>,
    pub per_sandbox: usize,
    pub total: usize,
    pub idle: Duration,
}

impl Default for TunnelPolicy {
    fn default() -> Self {
        Self {
            ports: vec![DEFAULT_PORTS],
            per_sandbox: DEFAULT_PER_SANDBOX,
            total: DEFAULT_TOTAL,
            idle: DEFAULT_IDLE,
        }
    }
}

impl TunnelPolicy {
    /// Reads the policy from the environment, or `None` naming the first unusable setting.
    ///
    /// # Errors
    ///
    /// Returns the name of the variable whose value is not usable.
    pub fn from_env() -> Result<Self, &'static str> {
        let mut policy = Self::default();
        if let Ok(value) = std::env::var(PORTS_ENV) {
            policy.ports = parse_ports(&value).ok_or(PORTS_ENV)?;
        }
        if let Ok(value) = std::env::var(PER_SANDBOX_ENV) {
            policy.per_sandbox = positive(&value).ok_or(PER_SANDBOX_ENV)?;
        }
        if let Ok(value) = std::env::var(TOTAL_ENV) {
            policy.total = positive(&value).ok_or(TOTAL_ENV)?;
        }
        if let Ok(value) = std::env::var(IDLE_ENV) {
            let seconds = positive(&value).ok_or(IDLE_ENV)?;
            policy.idle = Duration::from_secs(seconds as u64);
        }
        Ok(policy)
    }

    /// Whether `port` may be asked for.
    #[must_use]
    pub fn permits(&self, port: u16) -> bool {
        port != 0 && self.ports.iter().any(|range| range.contains(&port))
    }
}

fn positive(value: &str) -> Option<usize> {
    value.trim().parse().ok().filter(|number| *number > 0)
}

/// Parses `9223-9224,3000` into ranges; `None` for an empty list, port zero, or an empty range.
#[must_use]
pub fn parse_ports(value: &str) -> Option<Vec<RangeInclusive<u16>>> {
    let ranges: Option<Vec<_>> = value
        .split(',')
        .map(|part| {
            let part = part.trim();
            let (first, last) = part.split_once('-').unwrap_or((part, part));
            let first: u16 = first.trim().parse().ok()?;
            let last: u16 = last.trim().parse().ok()?;
            (first != 0 && first <= last).then_some(first..=last)
        })
        .collect();
    ranges.filter(|ranges| !ranges.is_empty())
}

/// Why a tunnel was refused before the guest was asked.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refused {
    PortNotAllowed,
    Limit,
}

/// One refusal as both faces render it: the loopback face's lowercase code and the public
/// runner's uppercase one travel together so the two cannot drift.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub status: u16,
    pub code: &'static str,
    pub public_code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
}

impl Refusal {
    /// The loopback face's error for this refusal.
    #[must_use]
    pub const fn api_error(self) -> ApiError {
        ApiError::new(self.status, self.code, self.message, self.retryable)
    }
}

impl From<Refused> for Refusal {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::PortNotAllowed => Self {
                status: 403,
                code: "port_not_allowed",
                public_code: "PORT_NOT_ALLOWED",
                message: "that guest port is not open to tunnels",
                retryable: false,
            },
            Refused::Limit => Self {
                status: 429,
                code: "tunnel_limit",
                public_code: "TUNNEL_LIMIT",
                message: "this sandbox or host holds as many tunnels as it allows",
                retryable: true,
            },
        }
    }
}

impl From<TunnelFailure> for Refusal {
    fn from(failure: TunnelFailure) -> Self {
        match failure {
            TunnelFailure::NotFound => Self {
                status: 404,
                code: "machine_not_found",
                public_code: "NOT_FOUND",
                message: "sandbox is not running on this host",
                retryable: false,
            },
            TunnelFailure::NoNetwork => Self {
                status: 422,
                code: "sandbox_has_no_network",
                public_code: "SANDBOX_HAS_NO_NETWORK",
                message: "this sandbox was created without a network",
                retryable: false,
            },
            TunnelFailure::PortNotAllowed => Refused::PortNotAllowed.into(),
            TunnelFailure::Unreachable => Self {
                status: 502,
                code: "guest_unreachable",
                public_code: "GUEST_UNREACHABLE",
                message: "the guest did not accept the connection",
                retryable: true,
            },
            TunnelFailure::Unavailable => Self {
                status: 503,
                code: "tunnel_unavailable",
                public_code: "TUNNEL_UNAVAILABLE",
                message: "the tunnel could not be opened",
                retryable: true,
            },
        }
    }
}

/// Counts open tunnels per sandbox and in total, and enforces the policy.
#[derive(Debug)]
pub struct TunnelTable {
    policy: TunnelPolicy,
    state: Mutex<TableState>,
}

#[derive(Debug, Default)]
struct TableState {
    per_sandbox: HashMap<String, usize>,
    total: usize,
}

impl TunnelTable {
    #[must_use]
    pub fn new(policy: TunnelPolicy) -> Arc<Self> {
        Arc::new(Self {
            policy,
            state: Mutex::new(TableState::default()),
        })
    }

    #[must_use]
    pub const fn policy(&self) -> &TunnelPolicy {
        &self.policy
    }

    /// Takes a place for one tunnel to `port` of sandbox `key`, or says why not.
    ///
    /// # Errors
    ///
    /// Returns [`Refused::PortNotAllowed`] for a port outside the policy and
    /// [`Refused::Limit`] when the sandbox or the process is full.
    pub fn admit(self: &Arc<Self>, key: &str, port: u16) -> Result<TunnelSlot, Refused> {
        if !self.policy.permits(port) {
            return Err(Refused::PortNotAllowed);
        }
        let mut state = self.lock();
        let held = state.per_sandbox.get(key).copied().unwrap_or(0);
        if held >= self.policy.per_sandbox || state.total >= self.policy.total {
            return Err(Refused::Limit);
        }
        state.per_sandbox.insert(key.to_owned(), held + 1);
        state.total += 1;
        Ok(TunnelSlot {
            table: Arc::clone(self),
            key: key.to_owned(),
        })
    }

    /// Tunnels `key` holds now.
    #[must_use]
    pub fn open_for(&self, key: &str) -> usize {
        self.lock().per_sandbox.get(key).copied().unwrap_or(0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TableState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One tunnel's place in the table; dropping it gives the place back.
#[derive(Debug)]
pub struct TunnelSlot {
    table: Arc<TunnelTable>,
    key: String,
}

impl Drop for TunnelSlot {
    fn drop(&mut self) {
        let mut state = self.table.lock();
        if let Some(held) = state.per_sandbox.get_mut(&self.key) {
            *held = held.saturating_sub(1);
            if *held == 0 {
                state.per_sandbox.remove(&self.key);
            }
        }
        state.total = state.total.saturating_sub(1);
    }
}

/// Whether an HTTP request asks for the tunnel upgrade: the `Upgrade` header names the token
/// and `Connection` lists `upgrade`.
#[must_use]
pub fn asks_for_upgrade(connection: Option<&str>, upgrade: Option<&str>) -> bool {
    let has = |value: Option<&str>, wanted: &str| {
        value.is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(wanted))
        })
    };
    has(connection, "upgrade") && has(upgrade, UPGRADE_TOKEN)
}

#[cfg(test)]
#[path = "tunnel_tests.rs"]
mod tests;
