//! The versioned protocols the local runtime speaks, for a binary's `--version` report.

use soma_version::Protocol;

/// Every protocol this build of the local runtime speaks, with the version it speaks.
///
/// The names are the ones `docs/product/reference/versioning.md` lists. Only the Linux KVM engine talks to a guest
/// agent and to the host daemons, so a build for any other target speaks none of them and its
/// report says so by listing nothing.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const PROTOCOLS: &[Protocol] = &[
    Protocol::new(
        "guest-application",
        soma_guest::APPLICATION_PROTOCOL_VERSION as u32,
    ),
    Protocol::new("guest-api", soma_guest::GUEST_API_LEVEL as u32),
    Protocol::new("launch-page", soma_guest::LAUNCH_PAGE_SCHEMA_VERSION as u32),
    Protocol::new("hostd", soma_hostd::protocol::PROTOCOL_VERSION as u32),
    Protocol::new("netd", soma_netd::PROTOCOL_VERSION as u32),
];

/// Every protocol this build of the local runtime speaks: none, off the Linux KVM engine.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub const PROTOCOLS: &[Protocol] = &[];
