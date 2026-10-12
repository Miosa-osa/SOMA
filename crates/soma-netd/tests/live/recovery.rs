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

/// A second broker with its own ledger and pin directory shares the host's links and tables, and
/// must reap nothing of the first one's: its sandboxes' objects are named like its own.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn a_second_broker_reaps_nothing_of_the_first_brokers_bundles() {
    use soma_netd::{NetNamespace, reap_unassigned, reconcile};

    live::require_privilege();
    let _world = World::build();
    let first_state = tempfile::tempdir().expect("first state dir");
    let second_state = tempfile::tempdir().expect("second state dir");
    let first = live::broker(first_state.path(), 16);
    let intent = super::checks::public_intent(&first);
    let (bundle, instance, operation) = live::ids(0x51);
    let sterile = first.prepare(bundle).expect("prepare");
    let assigned = first
        .assign(sterile, instance, operation, &intent, (5, broker_owner()))
        .map_err(|failure| failure.error)
        .expect("assign");
    // The first broker also holds an unassigned pool bundle, pinned in its own directory.
    let (pooled, _, _) = live::ids(0x52);
    std::mem::forget(first.prepare(pooled).expect("prepare pool bundle"));

    let second = live::broker(second_state.path(), 16);
    let report = reap_unassigned(&second).expect("second broker's reap");
    assert!(report.removed.is_empty(), "{report:?}");
    assert_eq!(
        report.foreign.len(),
        2,
        "both bundles are seen and left: {report:?}"
    );
    assert_eq!(
        NetNamespace::list(first.namespace_dir())
            .expect("pins")
            .len(),
        2,
        "the first broker's pins are untouched"
    );
    let after = reconcile(&first).expect("first broker reconcile");
    assert_eq!(after.entries.len(), 1);
    // The first broker's own reap would take its pool bundle and keep the assigned one.
    let own = reap_unassigned(&first).expect("first broker's own reap");
    assert_eq!(own.removed.len(), 1, "{own:?}");
    let released = soma_netd::release(&first, assigned);
    assert!(released.complete, "{released:?}");
}

/// How long the reap of a leaked pool takes, for the sizing of a start. Prints the figure.
#[test]
#[ignore = "requires CAP_NET_ADMIN inside the pinned privileged container"]
fn reaping_a_leaked_pool_finishes_in_a_bounded_time() {
    use soma_netd::{reap_plan, reap_unassigned};

    live::require_privilege();
    let _world = World::build();
    let state = tempfile::tempdir().expect("state dir");
    let first = live::broker(state.path(), 4096);
    let count: u8 = 160;
    for seed in 0..count {
        let (pooled, _, _) = live::ids(seed);
        std::mem::forget(first.prepare(pooled).expect("prepare pool bundle"));
    }
    let restarted = live::broker(state.path(), 4096);
    let planned = std::time::Instant::now();
    let plan = reap_plan(&restarted).expect("plan");
    let plan_ms = planned.elapsed().as_millis();
    assert_eq!(plan.candidates().len(), usize::from(count));
    let started = std::time::Instant::now();
    let report = plan.execute(&restarted, usize::MAX);
    let ms = started.elapsed().as_millis();
    println!(
        "reap of {count} leaked bundles: plan {plan_ms} ms, removal {ms} ms ({} ms each)",
        ms / u128::from(count)
    );
    assert_eq!(report.removed.len(), usize::from(count), "{report:?}");
    assert!(ms < 120_000, "reaping {count} bundles took {ms} ms");
    assert_eq!(reap_unassigned(&restarted).expect("again").removed.len(), 0);
}
