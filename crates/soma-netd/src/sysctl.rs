//! Per-namespace sysctl access through `/proc/sys/net`, which follows the calling thread's
//! network namespace.

use std::{fs, path::Path};

use crate::{Error, Step};

const IP_FORWARD: &str = "/proc/sys/net/ipv4/ip_forward";
const TCP_LOOSE: &str = "/proc/sys/net/netfilter/nf_conntrack_tcp_loose";
const DISABLE_IPV6: [&str; 2] = [
    "/proc/sys/net/ipv6/conf/all/disable_ipv6",
    "/proc/sys/net/ipv6/conf/default/disable_ipv6",
];

/// Reads whether IPv4 forwarding is enabled in the calling thread's namespace.
pub(crate) fn forwarding() -> Result<bool, Error> {
    let text = fs::read_to_string(IP_FORWARD).map_err(|error| Error::io(Step::Sysctl, &error))?;
    Ok(text.trim() == "1")
}

/// Sets IPv4 forwarding in the calling thread's namespace.
pub(crate) fn set_forwarding(enabled: bool) -> Result<(), Error> {
    write(IP_FORWARD, if enabled { "1\n" } else { "0\n" })
}

/// Sets whether conntrack in the calling thread's namespace adopts a TCP flow it did not see
/// begin.
///
/// With loose tracking on (the kernel default) a mid-stream packet creates an `established` entry,
/// so a flow the guest opened before its egress was closed would be admitted again as soon as
/// egress reopened. With it off such a packet is `invalid` and dropped, so a reopen admits only
/// flows that start after it. The setting is per namespace and disappears with the namespace.
///
/// # Errors
///
/// Returns [`Error::Kernel`] when the file is absent or cannot be written: a caller that needs
/// the guarantee must not proceed without it.
pub(crate) fn set_tcp_loose(enabled: bool) -> Result<(), Error> {
    write(TCP_LOOSE, if enabled { "1\n" } else { "0\n" })
}

/// Disables IPv6 for every current and future link in the calling thread's namespace.
///
/// A kernel without IPv6 has no such files and needs nothing disabled.
pub(crate) fn disable_ipv6() -> Result<(), Error> {
    for path in DISABLE_IPV6 {
        if Path::new(path).exists() {
            write(path, "1\n")?;
        }
    }
    Ok(())
}

/// Sets `route_localnet` on one link in the calling thread's namespace.
///
/// The setting is scoped to one link rather than to the whole namespace, so publishing a
/// loopback endpoint for one bundle changes nothing for any other link and the change
/// disappears with the veth at release instead of persisting as host state.
pub(crate) fn set_route_localnet(link: &str, enabled: bool) -> Result<(), Error> {
    let path = format!("/proc/sys/net/ipv4/conf/{link}/route_localnet");
    write(&path, if enabled { "1\n" } else { "0\n" })
}

fn write(path: &str, value: &str) -> Result<(), Error> {
    fs::write(path, value).map_err(|error| Error::io(Step::Sysctl, &error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwarding_is_readable_in_the_host_namespace() {
        let _ = forwarding().expect("readable");
    }
}
