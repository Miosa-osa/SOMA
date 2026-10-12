//! Recovery proofs for the broker's control protocol: a lost reply and an incomplete release.
//!
//! Both drive the real daemon over its `SOCK_SEQPACKET` control socket, so they observe the
//! exact accept, authentication, and handler path a jailed host process would.

use soma_netd::{ControlAuthority, Error, Reply, Request, broker_owner, error_code};

use super as live;
use super::{
    checks::public_intent_for,
    control::{Client, broker_on, current_group},
    session,
    world::World,
};

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_lost_activated_reply_is_recovered_rather_than_destroying_the_machine() {
    live::require_privilege();
    let world = World::build();
    let owner = broker_owner();
    let group = current_group();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    broker_on(
        state.path(),
        &socket,
        ControlAuthority::new(owner, group, &[owner], &[]).expect("lifecycle only"),
    );
    let client = Client::connect(&socket);
    let (_, instance, operation) = live::ids(0xe1);
    let intent = public_intent_for(&live::profile());

    client.send(&Request::Claim {
        instance,
        operation,
        vsock_cid: 9,
        intent: intent.clone(),
    });
    client.frame().expect("the transferred descriptor frame");
    let Some(Reply::Claimed {
        bundle,
        generation,
        activation,
        ..
    }) = client.reply()
    else {
        panic!("the claim must be answered with the assignment");
    };
    let receipt = session::repaired(*instance.as_bytes(), *operation.as_bytes())
        .network_activation(&activation, generation.get(), intent.digest().0)
        .expect("activation receipt");

    client.send(&Request::Activate {
        bundle,
        generation,
        receipt,
    });
    assert_eq!(client.reply(), Some(Reply::Activated));

    // The peer never saw that reply and replays the identical request.
    client.send(&Request::Activate {
        bundle,
        generation,
        receipt,
    });
    assert_eq!(
        client.reply(),
        Some(Reply::Activated),
        "a replayed activation must return the committed result, not destroy the Machine"
    );

    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: true }),
        "the assignment must still have been live to release"
    );

    // The record is now only in the ledger; the peer it records may still replay its release.
    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: true }),
        "the recorded owner must be able to replay an idempotent release"
    );
    drop(world);
}

#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn an_incomplete_release_keeps_its_operation_identity_reserved() {
    live::require_privilege();
    let world = World::build();
    let owner = broker_owner();
    let group = current_group();
    let state = tempfile::tempdir().expect("state dir");
    let run = tempfile::tempdir().expect("run dir");
    let socket = run.path().join("host").join("broker.sock");
    broker_on(
        state.path(),
        &socket,
        ControlAuthority::new(owner, group, &[owner], &[]).expect("lifecycle only"),
    );
    let client = Client::connect(&socket);
    let (_, instance, operation) = live::ids(0xe2);
    let intent = public_intent_for(&live::profile());
    let claim = Request::Claim {
        instance,
        operation,
        vsock_cid: 11,
        intent,
    };

    client.send(&claim);
    client.frame().expect("the transferred descriptor frame");
    let Some(Reply::Claimed {
        bundle, generation, ..
    }) = client.reply()
    else {
        panic!("the claim must be answered with the assignment");
    };

    // Break the namespace pin so the teardown cannot enter it and the release is incomplete.
    live::break_namespace_pin(&state.path().join("ns").join(bundle.short_hex()));
    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: false }),
        "a teardown that could not enter the namespace must report an incomplete release"
    );

    client.send(&claim);
    assert_eq!(
        client.reply(),
        Some(Reply::Failed(error_code(&Error::NotAssigned))),
        "an operation whose release left kernel objects behind must not take a second lease"
    );

    // The first release already removed the broken pin, because a failing step no longer
    // abandons the steps after it; the retry finishes what that failure left behind.
    client.send(&Request::Release { bundle, generation });
    assert_eq!(
        client.reply(),
        Some(Reply::Released { complete: true }),
        "the retried release must finish the steps the first one could not"
    );
    client.send(&claim);
    client.frame().expect("the transferred descriptor frame");
    let Some(Reply::Claimed {
        bundle, generation, ..
    }) = client.reply()
    else {
        panic!("a completed release must free the operation identity");
    };
    client.send(&Request::Release { bundle, generation });
    assert_eq!(client.reply(), Some(Reply::Released { complete: true }));
    drop(world);
}

/// A restart reaps the pool the previous process prepared and never assigned, however many
/// restarts there are, and leaves a bundle the ledger owns alone.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn restarts_reap_the_unassigned_pool_and_keep_assigned_bundles() {
    use soma_netd::{Disposition, NetNamespace, reap_unassigned, reconcile};

    live::require_privilege();
    let _world = World::build();
    let state = tempfile::tempdir().expect("state dir");
    let first = live::broker(state.path(), 16);
    let intent = super::checks::public_intent(&first);
    let (bundle, instance, operation) = live::ids(0xa1);
    let sterile = first.prepare(bundle).expect("prepare the assigned bundle");
    let assigned = first
        .assign(sterile, instance, operation, &intent, (5, broker_owner()))
        .map_err(|failure| failure.error)
        .expect("assign");

    for restart in 0..3_u8 {
        // The process that prepared this pool ends without releasing it.
        for seed in 0..4_u8 {
            let (pooled, _, _) = live::ids(0x10 + restart * 8 + seed);
            std::mem::forget(first.prepare(pooled).expect("prepare pool bundle"));
        }
        let restarted = live::broker(state.path(), 16);
        let before = reconcile(&restarted).expect("reconcile before");
        assert_eq!(before.unowned_namespaces.len(), 4, "{before:?}");

        let report = reap_unassigned(&restarted).expect("reap");
        assert_eq!(report.removed.len(), 4, "{report:?}");
        assert!(
            report.in_use.is_empty() && report.failed.is_empty(),
            "{report:?}"
        );
        let after = reconcile(&restarted).expect("reconcile after");
        assert_eq!(after.unowned(), 0, "{after:?}");
        assert_eq!(after.entries.len(), 1);
        assert_eq!(after.entries[0].2, Disposition::Consistent);
        assert_eq!(
            NetNamespace::list(restarted.namespace_dir())
                .expect("pins")
                .len(),
            1,
            "only the ledger's bundle keeps a pin"
        );
        assert_eq!(
            reap_unassigned(&restarted)
                .expect("second reap")
                .removed
                .len(),
            0,
            "reaping is idempotent"
        );
    }
    let released = soma_netd::release(&first, assigned);
    assert!(released.complete, "{released:?}");
}

/// A bundle whose namespace a process still has open is never reaped.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_bundle_with_a_process_inside_its_namespace_is_not_reaped() {
    use soma_netd::{NetNamespace, reap_unassigned};

    live::require_privilege();
    let _world = World::build();
    let state = tempfile::tempdir().expect("state dir");
    let first = live::broker(state.path(), 16);
    let (pooled, _, _) = live::ids(0x31);
    std::mem::forget(first.prepare(pooled).expect("prepare"));
    let restarted = live::broker(state.path(), 16);
    let pin = NetNamespace::list(restarted.namespace_dir()).expect("pins");
    assert_eq!(pin.len(), 1);
    let path = restarted.namespace_dir().join(&pin[0]);
    let mut holder = std::process::Command::new("nsenter")
        .arg(format!("--net={}", path.display()))
        .args(["sleep", "60"])
        .spawn()
        .expect("nsenter holds the namespace");
    std::thread::sleep(std::time::Duration::from_millis(500));

    let held = reap_unassigned(&restarted).expect("reap while held");
    assert_eq!(held.in_use, pin, "{held:?}");
    assert!(held.removed.is_empty(), "{held:?}");
    assert!(path.exists(), "the held bundle keeps its pin");

    holder.kill().expect("stop the holder");
    holder.wait().expect("reap the holder");
    let released = reap_unassigned(&restarted).expect("reap after");
    assert_eq!(released.removed, pin, "{released:?}");
}

/// A ledger that cannot be read proves nothing about ownership, so nothing is removed.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn an_unreadable_ledger_removes_nothing() {
    use soma_netd::{NetNamespace, reap_unassigned};

    live::require_privilege();
    let _world = World::build();
    let state = tempfile::tempdir().expect("state dir");
    let first = live::broker(state.path(), 16);
    let (pooled, _, _) = live::ids(0x41);
    std::mem::forget(first.prepare(pooled).expect("prepare"));
    let restarted = live::broker(state.path(), 16);
    // An assignment record whose content does not decode.
    std::fs::write(
        state
            .path()
            .join("ledger")
            .join("00000000000000000000000000000001-00000001.assigned"),
        b"x",
    )
    .expect("plant a corrupt entry");

    assert!(
        reap_unassigned(&restarted).is_err(),
        "a ledger that cannot be read must stop the reap"
    );
    assert_eq!(
        NetNamespace::list(restarted.namespace_dir())
            .expect("pins")
            .len(),
        1,
        "the unowned bundle is still there"
    );
    std::fs::remove_file(
        state
            .path()
            .join("ledger")
            .join("00000000000000000000000000000001-00000001.assigned"),
    )
    .expect("remove the corrupt entry");
    let released = reap_unassigned(&restarted).expect("reap once the ledger reads");
    assert_eq!(released.removed.len(), 1, "{released:?}");
}

/// A link or table that carries the broker's naming but has no pin in this broker's directory may
/// belong to another broker, and is reported, never removed.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn another_brokers_link_and_table_survive_the_reap() {
    use std::process::Command;

    use soma_netd::reap_unassigned;

    live::require_privilege();
    let _world = World::build();
    let state = tempfile::tempdir().expect("state dir");
    let broker = live::broker(state.path(), 16);
    let run = |program: &str, arguments: &[&str]| {
        let status = Command::new(program)
            .args(arguments)
            .status()
            .expect(program);
        assert!(status.success(), "{program} {arguments:?}");
    };
    run(
        "ip",
        &[
            "link",
            "add",
            "svfeed0001",
            "type",
            "veth",
            "peer",
            "name",
            "svfeed0002",
        ],
    );
    run("nft", &["add", "table", "inet", "somah_feed0001"]);

    let report = reap_unassigned(&broker).expect("reap");
    let linked = Command::new("ip")
        .args(["link", "show", "svfeed0001"])
        .status()
        .expect("ip")
        .success();
    let tabled = Command::new("nft")
        .args(["list", "table", "inet", "somah_feed0001"])
        .status()
        .expect("nft")
        .success();
    run("ip", &["link", "del", "svfeed0001"]);
    run("nft", &["delete", "table", "inet", "somah_feed0001"]);
    assert!(report.removed.is_empty(), "{report:?}");
    assert_eq!(
        report.foreign,
        ["feed0001", "feed0002"],
        "both ends of the other broker's veth are reported, none removed: {report:?}"
    );
    assert!(linked && tabled, "the other broker's objects were removed");
}
