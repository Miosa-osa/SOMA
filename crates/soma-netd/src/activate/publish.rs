//! The single step that makes a published port reachable.
//!
//! Assignment has already reserved the host endpoints and rendered the sandbox ruleset that
//! will admit them, but neither reaches the guest on its own: no translation exists, the host
//! table drops every unsolicited packet toward the veth, and forwarding is off. Installing the
//! publication table is therefore the counterpart of enabling forwarding, and it runs last,
//! after the links are up and the routes exist, so a mapping is never live before the path
//! behind it is.

use crate::{Assigned, Error, PublicationRuleset, nft, sysctl};

/// Installs every published mapping of one activated assignment.
///
/// Returns the operator-facing description of each mapping, in the order it was installed.
pub(super) fn install(assigned: &Assigned) -> Result<Vec<String>, Error> {
    let published = &assigned.published;
    if published.is_empty() {
        return Ok(Vec::new());
    }
    let names = &assigned.bundle.names;
    // A loopback publication is answered on the bundle's host veth with a `127.0.0.0/8`
    // destination, because conntrack reverses the translation before the routing decision, and
    // the kernel discards that as a martian unless this link is told otherwise. The setting is
    // scoped to the link the bundle owns, so it disappears with the veth at release.
    if published.iter().any(super::PublishedPort::binds_loopback) {
        sysctl::set_route_localnet(&names.host_veth, true)?;
    }
    nft::apply(&PublicationRuleset { names, published }.render())?;
    Ok(published
        .iter()
        .map(super::PublishedPort::describe)
        .collect())
}
