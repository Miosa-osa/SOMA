//! The live proof that a restart cannot leave a window open that nobody recorded.
//!
//! A broker that restarts has lost every window it opened, while the kernel keeps the rulesets.
//! The new broker puts each bundle claimed with no egress back to its base and leaves a bundle
//! whose base allows egress alone, and reconcile reports an open ruleset with no recorded window.

use soma_netd::{EgressClass, activate, broker_owner, release};

use super::{GATEWAY_WAIT, bring_up, sandbox_flows};
use crate::live::{
    self,
    checks::{PROBE, transfer_tap},
    frames::SynOutcome,
    session, world,
};

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_restarted_broker_puts_a_denied_bundle_back_to_its_base_and_leaves_a_public_one_alone() {
    live::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let first = live::broker(state.path(), 16);
    let (mut denied, mut denied_guest) = bring_up(&first, 0xe5, true);

    // A public bundle shares the ledger: its base allows egress and must survive the restart.
    let (public_bundle, public_instance, public_operation) = live::ids(0xe6);
    let sterile = first.prepare(public_bundle).expect("prepare public");
    let public_intent = live::checks::public_intent(&first);
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
    let second = live::broker(state.path(), 16);
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

    live::require_privilege();
    let _world = world::World::build();
    let state = tempfile::tempdir().expect("state dir");
    let broker = live::broker(state.path(), 16);
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
