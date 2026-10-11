//! The kernel work behind [`super::Broker::prepare`].

use std::os::fd::{AsFd, OwnedFd};

use sha2::{Digest, Sha256};

use super::Broker;
use crate::{
    BundleId, BundleNames, Cidr, ConntrackZone, Error, HostRuleset, LeasePair, SandboxRuleset,
    derive_macs, link,
    namespace::NetNamespace,
    netlink, nft, sysctl,
    tap::{self, TapSpec},
};

/// The digest that decides whether an assignment may reuse the committed ruleset.
pub(super) fn ruleset_digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Builds every kernel object of one sterile bundle and returns the TAP descriptor and the
/// digest of the sandbox ruleset it committed.
///
/// Order: veth pair with the peer placed into the namespace, then inside the namespace IPv6
/// off, forwarding off, TAP with the gateway address, sandbox veth address, and the sandbox
/// ruleset rendered for the prewarm intent; then the host veth address and the host zone and
/// masquerade table. The links stay down and forwarding stays off until activation, so the
/// ruleset admitting public egress cannot carry a packet before then.
pub(super) fn build(
    broker: &Broker,
    names: &BundleNames,
    namespace: &NetNamespace,
    leases: LeasePair,
    zone: ConntrackZone,
    id: BundleId,
) -> Result<(OwnedFd, [u8; 32]), Error> {
    let macs = derive_macs(id);
    let tap_name = names.tap.clone();
    let veth_name = names.sandbox_veth.clone();
    let sandbox = SandboxRuleset {
        names,
        lease: leases.guest,
        guest_mac: macs.guest,
        intent: broker.prewarm_intent(),
        protected: broker.profile().protected(),
        published: &[],
        connect_ports: &[],
    }
    .render();
    let ruleset = ruleset_digest(&sandbox);
    netlink::create_veth(
        &names.host_veth,
        &names.sandbox_veth,
        namespace.as_fd().as_fd(),
    )?;
    let tap = namespace.within(move || {
        sysctl::disable_ipv6()?;
        sysctl::set_forwarding(false)?;
        let tap = tap::create(TapSpec {
            name: &tap_name,
            mac: macs.tap,
            gateway: leases.guest.host(),
            prefix: leases.guest.prefix_length(),
        })?;
        let socket = link::control_socket()?;
        link::set_address(
            &socket,
            &veth_name,
            leases.transit.guest(),
            leases.transit.prefix_length(),
        )?;
        nft::apply(&sandbox)?;
        // Raised here, off the request path: every link flag change and route install takes the
        // kernel's network configuration lock, and a hundred activations queueing for it were
        // the slowest part of a burst. Traffic still cannot flow, because forwarding in the
        // namespace stays off until activation, and the guest has no session to repair
        // through until a Machine holds the other end of this TAP.
        link::set_up(&socket, &tap_name, true)?;
        link::set_up(&socket, &veth_name, true)?;
        link::add_route(
            &socket,
            Cidr::V4(std::net::Ipv4Addr::UNSPECIFIED, 0),
            leases.transit.host(),
        )?;
        Ok(tap)
    })?;
    let socket = link::control_socket()?;
    link::set_address(
        &socket,
        &names.host_veth,
        leases.transit.host(),
        leases.transit.prefix_length(),
    )?;
    link::set_up(&socket, &names.host_veth, true)?;
    link::add_route(&socket, leases.guest.cidr(), leases.transit.guest())?;
    nft::apply(
        &HostRuleset {
            names,
            lease: leases.guest,
            uplink: broker.profile().uplink(),
            zone,
            protected: broker.profile().protected(),
        }
        .render(),
    )?;
    Ok((tap, ruleset))
}
