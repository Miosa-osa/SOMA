//! The live proof that one activated bundle's egress can be opened and closed without a restart.
//!
//! A bundle is claimed with a denied intent, activated, and shown to reach nothing. Opening it
//! reaches the public destination and the declared resolver; closing it drops both again and
//! leaves no tracked flow behind in the sandbox namespace or the host zone. Asking for DNS
//! without egress, changing a bundle that is not activated, and changing one whose namespace is
//! gone are refused rather than reported as done.

use std::{net::Ipv4Addr, process::Command, time::Duration};

use soma_netd::{
    Assigned, Broker, CleanupGeneration, EgressClass, Error, IntentRejection, InterfaceName,
    NetworkIntent, NetworkProfile, SubnetPlan, activate, broker_owner, release,
};

use super::{
    checks::{PROBE, transfer_tap},
    frames::{Guest, SynOutcome},
    session, world,
};

const CONNTRACK: &str = "/usr/sbin/conntrack";
const GATEWAY_WAIT: Duration = Duration::from_secs(5);

fn bring_up(broker: &Broker, seed: u8, activated: bool) -> (Assigned, Guest) {
    let (bundle, instance, operation) = super::ids(seed);
    let sterile = broker.prepare(bundle).expect("prepare");
    let intent = NetworkIntent::denied(broker.profile());
    let mut assigned = broker
        .assign(sterile, instance, operation, &intent, (7, broker_owner()))
        .map_err(|failure| failure.error)
        .expect("assign");
    let mut guest = transfer_tap(&assigned);
    if activated {
        let host = session::repaired(*instance.as_bytes(), *operation.as_bytes());
        let receipt = session::mint(&host, &assigned);
        activate(&mut assigned, &receipt).expect("activate");
        assert!(guest.resolve_gateway(GATEWAY_WAIT).is_some(), "gateway ARP");
    }
    (assigned, guest)
}

/// Counts the tracked flows of the calling namespace, or of one zone when `zone` is given.
fn tracked(zone: Option<u16>) -> u32 {
    let mut command = Command::new(CONNTRACK);
    command.arg("-C");
    if let Some(zone) = zone {
        command.args(["-w", &zone.to_string()]);
    }
    let output = command.output().expect("conntrack binary");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("conntrack count")
}

fn sandbox_flows(assigned: &Assigned) -> u32 {
    assigned
        .bundle()
        .namespace()
        .within(|| Ok(tracked(None)))
        .expect("count sandbox flows")
}

fn zone_flows(assigned: &Assigned) -> u32 {
    tracked(Some(assigned.bundle().zone().get()))
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
#[allow(clippy::too_many_lines)]
fn egress_opens_and_closes_an_activated_bundle_and_a_close_leaves_no_tracked_flow() {
    super::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let broker = super::broker(state.path(), 16);
    let (mut assigned, mut guest) = bring_up(&broker, 0xe1, true);

    // Claimed denied: neither TCP nor DNS leaves the sandbox.
    assert_eq!(
        guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::Silence,
        "a bundle claimed with a denied intent reaches nothing"
    );
    assert_eq!(guest.udp_probe(world::DECLARED_RESOLVER, 53, PROBE), None);

    // Open with DNS.
    let opened = broker
        .set_egress(&mut assigned, EgressClass::PublicInternet, true)
        .expect("open");
    assert!(opened.changed);
    assert_eq!(
        guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::SynAck,
        "an opened bundle reaches the public destination"
    );
    assert_eq!(
        guest
            .udp_probe(world::DECLARED_RESOLVER, 53, PROBE)
            .as_deref(),
        Some(&b"dns-ok"[..]),
        "an opened bundle with DNS reaches the declared resolver"
    );
    // The protected set still applies to an opened bundle.
    assert_eq!(
        guest.tcp_syn(world::METADATA, 80, PROBE),
        SynOutcome::Silence
    );
    assert!(
        sandbox_flows(&assigned) > 0,
        "the DNS exchange is tracked in the sandbox namespace"
    );
    assert!(zone_flows(&assigned) > 0, "and in the bundle's host zone");

    // The same state again is a no-op.
    let again = broker
        .set_egress(&mut assigned, EgressClass::PublicInternet, true)
        .expect("open again");
    assert!(!again.changed);

    // Close.
    let closed = broker
        .set_egress(&mut assigned, EgressClass::Denied, false)
        .expect("close");
    assert!(closed.changed);
    assert_eq!(
        guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::Silence,
        "a closed bundle reaches nothing again"
    );
    assert_eq!(
        guest.udp_probe(world::DECLARED_RESOLVER, 53, PROBE),
        None,
        "DNS is blocked after close"
    );
    assert_eq!(sandbox_flows(&assigned), 0, "close flushes the sandbox");
    assert_eq!(zone_flows(&assigned), 0, "close flushes the host zone");

    // Closing a closed bundle still succeeds, and still leaves nothing tracked.
    let closed_again = broker
        .set_egress(&mut assigned, EgressClass::Denied, false)
        .expect("close again");
    assert!(!closed_again.changed);
    assert_eq!(sandbox_flows(&assigned), 0);

    // DNS needs egress.
    assert_eq!(
        broker
            .set_egress(&mut assigned, EgressClass::Denied, true)
            .map(|_| ()),
        Err(Error::InvalidIntent(IntentRejection::DnsWithoutEgress))
    );

    drop(guest);
    assert!(release(&broker, assigned).complete);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_flow_opened_before_a_close_does_not_come_back_when_egress_reopens() {
    super::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let broker = super::broker(state.path(), 16);
    let (mut assigned, mut guest) = bring_up(&broker, 0xe4, true);

    broker
        .set_egress(&mut assigned, EgressClass::PublicInternet, false)
        .expect("open");
    let flow = guest
        .tcp_open(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE)
        .expect("a flow opened inside the window");
    assert!(
        guest.tcp_data_answered(&flow, PROBE),
        "the flow is admitted while the window is open"
    );

    broker
        .set_egress(&mut assigned, EgressClass::Denied, false)
        .expect("close");
    assert!(
        !guest.tcp_data_answered(&flow, PROBE),
        "the flow is cut while egress is closed"
    );

    // Reopened, the same flow must not be adopted mid-stream: only a flow that starts after the
    // reopen is admitted. With conntrack's loose tracking on, this segment would be accepted.
    broker
        .set_egress(&mut assigned, EgressClass::PublicInternet, false)
        .expect("reopen");
    assert!(
        !guest.tcp_data_answered(&flow, PROBE),
        "a flow from an earlier window is not adopted after a reopen"
    );
    assert_eq!(
        guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::SynAck,
        "a fresh flow is admitted after the reopen"
    );
    drop(guest);
    assert!(release(&broker, assigned).complete);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_bundle_that_is_not_activated_refuses_an_egress_change() {
    super::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let broker = super::broker(state.path(), 16);
    let (mut assigned, guest) = bring_up(&broker, 0xe2, false);

    assert!(matches!(
        broker.set_egress(&mut assigned, EgressClass::PublicInternet, true),
        Err(Error::InvalidState(_))
    ));
    drop(guest);
    assert!(release(&broker, assigned).complete);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_flip_that_cannot_be_applied_is_an_error_and_leaves_the_bundle_closed() {
    super::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    // A profile that names no resolver cannot honour a request for DNS.
    let profile = NetworkProfile::new(
        InterfaceName::new(world::UPLINK).expect("uplink"),
        SubnetPlan::new(Ipv4Addr::new(10, 200, 0, 0), 16).expect("leases"),
        SubnetPlan::new(Ipv4Addr::new(10, 201, 0, 0), 16).expect("transit"),
        Vec::new(),
        &[world::HOST_ADDRESS.into()],
        &[],
    )
    .expect("profile");
    let broker = Broker::open(
        profile,
        state.path(),
        CleanupGeneration::new(1).expect("generation"),
        16,
    )
    .expect("broker");
    let (mut assigned, mut guest) = bring_up(&broker, 0xe3, true);

    assert!(matches!(
        broker.set_egress(&mut assigned, EgressClass::PublicInternet, true),
        Err(Error::InvalidState(_))
    ));
    assert_eq!(
        guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::Silence,
        "a refused open leaves the bundle closed"
    );
    // Without DNS the same profile opens.
    assert!(
        broker
            .set_egress(&mut assigned, EgressClass::PublicInternet, false)
            .expect("open without dns")
            .changed
    );
    drop(guest);
    assert!(release(&broker, assigned).complete);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_restarted_broker_puts_a_denied_bundle_back_to_its_base_and_leaves_a_public_one_alone() {
    super::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let first = super::broker(state.path(), 16);
    let (mut denied, mut denied_guest) = bring_up(&first, 0xe5, true);

    // A public bundle shares the ledger: its base allows egress and must survive the restart.
    let (public_bundle, public_instance, public_operation) = super::ids(0xe6);
    let sterile = first.prepare(public_bundle).expect("prepare public");
    let public_intent = super::checks::public_intent(&first);
    let mut public = first
        .assign(
            sterile,
            public_instance,
            public_operation,
            &public_intent,
            (7, broker_owner()),
        )
        .map_err(|failure| failure.error)
        .expect("assign public");
    let mut public_guest = transfer_tap(&public);
    let host = session::repaired(*public_instance.as_bytes(), *public_operation.as_bytes());
    let receipt = session::mint(&host, &public);
    activate(&mut public, &receipt).expect("activate public");
    assert!(public_guest.resolve_gateway(GATEWAY_WAIT).is_some());

    first
        .set_egress(&mut denied, EgressClass::PublicInternet, true)
        .expect("open");
    assert_eq!(
        denied_guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::SynAck,
        "the window is open before the restart"
    );

    // The broker restarts: its memory of the window is gone, the kernel keeps the ruleset. A new
    // broker over the same state is the same situation.
    let second = super::broker(state.path(), 16);
    let report = soma_netd::restore_base_rulesets(&second).expect("restore");
    assert_eq!(report.restored, vec![denied.record().bundle.short_hex()]);
    assert!(report.forwarding_off.is_empty() && report.failed.is_empty());

    assert_eq!(
        denied_guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::Silence,
        "the restart closed the window"
    );
    assert_eq!(
        public_guest.tcp_syn(world::PUBLIC_ADDRESS, world::PUBLIC_PORT, PROBE),
        SynOutcome::SynAck,
        "a bundle whose base allows egress keeps it"
    );
    assert_eq!(sandbox_flows(&denied), 0);

    drop((denied_guest, public_guest));
    assert!(release(&first, denied).complete);
    assert!(release(&first, public).complete);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn reconcile_reports_an_open_ruleset_nobody_recorded_and_accepts_one_that_is_recorded() {
    use soma_netd::{Disposition, Drift, reconcile_with};

    super::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let broker = super::broker(state.path(), 16);
    let (mut assigned, guest) = bring_up(&broker, 0xe7, true);
    let bundle = assigned.record().bundle;
    let generation = assigned.record().generation;
    let disposition = |report: soma_netd::ReconcileReport| {
        report
            .entries
            .into_iter()
            .find(|(id, _, _)| *id == bundle)
            .map(|(_, _, disposition)| disposition)
    };

    // Closed: consistent, whatever the caller records.
    assert_eq!(
        disposition(reconcile_with(&broker, &|_, _| false).expect("reconcile")),
        Some(Disposition::Consistent)
    );

    broker
        .set_egress(&mut assigned, EgressClass::PublicInternet, false)
        .expect("open");
    // Open and nobody recorded a window: drifted, which is what a restart that lost the record
    // would look like.
    assert_eq!(
        disposition(reconcile_with(&broker, &|_, _| false).expect("reconcile")),
        Some(Disposition::Drifted(Drift::RulesetNotBase))
    );
    // Open and recorded: the window is what was asked for.
    assert_eq!(
        disposition(
            reconcile_with(&broker, &|id, g| id == bundle && g == generation).expect("reconcile")
        ),
        Some(Disposition::Consistent)
    );
    drop(guest);
    assert!(release(&broker, assigned).complete);
}
