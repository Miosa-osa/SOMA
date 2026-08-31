//! The live proof that a published port is reachable only between activation and release.
//!
//! The intent used here denies egress and DNS, so the only thing that can carry a packet to
//! the guest is the publication itself. The same host endpoint is probed three times, from
//! outside the broker, with nothing but an ordinary TCP connect: once while the bundle is
//! merely assigned, once after activation, and once after release.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream},
    process::Command,
    time::Duration,
};

use soma::{DnsPolicy, EgressPolicy, NetworkPolicy, PortPublication};
use soma_netd::{NetworkIntent, StepResult, activate, broker_owner, release};

use super::{
    checks::transfer_tap,
    session::{self},
};

/// The guest port the stand-in answers on.
const SERVICE: u16 = 80;

/// The bound wait for one connection attempt.
///
/// A refusal is immediate, and a success costs one translated round trip plus, on the first
/// attempt, the sandbox kernel's neighbour lookup, so this is generous rather than tight.
const CONNECT: Duration = Duration::from_secs(3);

/// Runs the whole reachability sequence for one loopback TCP publication.
pub fn reachable_only_after_activation() {
    let state = tempfile::tempdir().expect("state dir");
    let mut broker = super::broker(state.path(), 16);
    let policy = NetworkPolicy::new(
        EgressPolicy::Denied,
        DnsPolicy::Denied,
        vec![PortPublication::loopback_tcp(SERVICE).expect("publication")],
    )
    .expect("policy");
    let intent = NetworkIntent::admit(&policy, broker.profile()).expect("admitted");

    let (bundle, instance, operation) = super::ids(0xe1);
    let sterile = broker.prepare(bundle).expect("prepare");
    let mut assigned = broker
        .assign(sterile, instance, operation, &intent, (7, broker_owner()))
        .map_err(|failure| failure.error)
        .expect("assign");
    let published = assigned.published();
    assert_eq!(published.len(), 1, "one mapping per publication");
    let endpoint = SocketAddr::from((Ipv4Addr::LOCALHOST, published[0].host_port()));
    assert_eq!(published[0].guest_port(), SERVICE);
    assert!(
        !publication_tables().contains(&table_name(bundle)),
        "an assigned bundle installs no translation"
    );
    assert!(
        TcpStream::connect_timeout(&endpoint, CONNECT).is_err(),
        "a reserved but unactivated publication must refuse the connection"
    );

    let mut guest = transfer_tap(&assigned);
    let host = session::repaired(*instance.as_bytes(), *operation.as_bytes());
    let receipt = session::mint(&host, &assigned);
    let evidence = activate(&mut assigned, &receipt).expect("activate");
    assert_eq!(
        evidence.published.len(),
        1,
        "activation must report the mapping it installed: {evidence:?}"
    );
    assert!(
        publication_tables().contains(&table_name(bundle)),
        "activation installs the publication table"
    );
    // The stand-in announces itself first so the sandbox kernel already holds its address,
    // which keeps the first translated packet from waiting on a neighbour lookup.
    assert!(
        guest.resolve_gateway(CONNECT).is_some(),
        "gateway ARP after activation"
    );
    let client = std::thread::spawn(move || TcpStream::connect_timeout(&endpoint, CONNECT));
    let source = guest.accept_syn(SERVICE, CONNECT);
    let connected = client.join().expect("client thread");
    assert_eq!(
        source,
        Some(transit_host(&assigned)),
        "the client's source must reach the guest translated into the bundle's transit address"
    );
    assert!(
        connected.is_ok(),
        "the published port must be reachable after activation: {connected:?}"
    );
    drop(connected);
    drop(guest);

    let released = release(&broker, assigned);
    assert_eq!(
        released.published,
        StepResult::Removed,
        "release must remove the publication table it installed: {released:?}"
    );
    assert!(released.complete && released.ledger, "{released:?}");
    assert!(
        !publication_tables().contains(&table_name(bundle)),
        "no translation may outlive the Instance"
    );
    assert!(
        TcpStream::connect_timeout(&endpoint, CONNECT).is_err(),
        "a released publication must be unreachable again"
    );
}

/// Returns the host end of the bundle's transit pair, which is what the masquerade rewrites
/// every published client's source into.
fn transit_host(assigned: &soma_netd::Assigned) -> Ipv4Addr {
    assigned.bundle().leases().transit.host()
}

fn table_name(bundle: soma_netd::BundleId) -> String {
    format!("somap_{}", bundle.short_hex())
}

/// Lists the publication tables the kernel holds, read through the pinned tool rather than
/// through the broker, so the proof does not depend on the code under test.
fn publication_tables() -> Vec<String> {
    let output = Command::new("/usr/sbin/nft")
        .args(["list", "tables"])
        .output()
        .expect("nft binary");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter(|word| word.starts_with("somap_"))
        .map(ToOwned::to_owned)
        .collect()
}
