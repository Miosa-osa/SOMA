//! The kernel work behind [`super::Broker::prepare`].

use std::os::fd::{AsFd, OwnedFd};

use super::Broker;
use crate::{
    BundleId, BundleNames, ConntrackZone, Error, HostRuleset, LeasePair, NetworkIntent,
    SandboxRuleset, derive_macs, link,
    namespace::NetNamespace,
    netlink, nft, sysctl,
    tap::{self, TapSpec},
};

/// Builds every kernel object of one sterile bundle and returns the TAP descriptor.
///
/// Order: veth pair with the peer placed into the namespace, then inside the namespace IPv6
/// off, forwarding off, TAP with the gateway address, sandbox veth address, and the denied
/// ruleset; then the host veth address and the host zone and masquerade table.
pub(super) fn build(
    broker: &Broker,
    names: &BundleNames,
    namespace: &NetNamespace,
    leases: LeasePair,
    zone: ConntrackZone,
    id: BundleId,
) -> Result<OwnedFd, Error> {
    let macs = derive_macs(id);
    let denied = NetworkIntent::denied(broker.profile());
    let tap_name = names.tap.clone();
    let veth_name = names.sandbox_veth.clone();
    let sandbox = SandboxRuleset {
        names,
        lease: leases.guest,
        guest_mac: macs.guest,
        intent: &denied,
        protected: broker.profile().protected(),
    }
    .render();
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
        Ok(tap)
    })?;
    let socket = link::control_socket()?;
    link::set_address(
        &socket,
        &names.host_veth,
        leases.transit.host(),
        leases.transit.prefix_length(),
    )?;
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
    Ok(tap)
}
