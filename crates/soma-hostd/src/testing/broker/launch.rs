//! Descriptor pairs, sterile head tokens, and derived launch identities for the broker.

use std::os::{fd::OwnedFd, unix::net::UnixStream};

use soma_guest::LaunchNetwork;
use soma_netd::{BundleId, CleanupGeneration, derive_macs};
use soma_storage::HeadToken;

use crate::{Resource, ResourceFault, ResourceFaultKind, WorkerId, pool::ledger::now_nanos};

pub(super) fn pair(resource: Resource) -> Result<OwnedFd, ResourceFault> {
    UnixStream::pair()
        .map(|(ours, _theirs)| OwnedFd::from(ours))
        .map_err(|_| ResourceFault {
            resource,
            kind: ResourceFaultKind::Failed,
        })
}

pub(super) fn sterile_token(worker: WorkerId) -> HeadToken {
    HeadToken::new(*worker.as_bytes()).unwrap_or_else(|_| unreachable!("worker ids are nonzero"))
}

/// Derives the launch-page network identity of one assigned bundle.
pub(super) fn launch_identity(
    bundle: BundleId,
    generation: CleanupGeneration,
    vsock_cid: u32,
    index: u32,
) -> Result<LaunchNetwork, ResourceFault> {
    let index = index % 16_000;
    let host = 4 * (index % 64) + 1;
    let third = 1 + (index / 64) % 250;
    let gateway = [
        10,
        200,
        u8::try_from(third).unwrap_or(1),
        u8::try_from(host).unwrap_or(1),
    ];
    let address = [gateway[0], gateway[1], gateway[2], gateway[3] + 1];
    LaunchNetwork::new(
        vsock_cid,
        generation.get(),
        derive_macs(bundle).guest,
        address,
        30,
        gateway,
        gateway,
        now_nanos(),
    )
    .map_err(|_| ResourceFault {
        resource: Resource::Network,
        kind: ResourceFaultKind::Denied,
    })
}
