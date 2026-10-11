//! Concurrent claim, activate, and release through the real daemon, with raw samples.
//!
//! The daemon runs on a thread of this process and every client speaks the control socket
//! protocol, so the numbers include the accept, authentication, thread hand-off, descriptor
//! transfer, and ledger commit a sandbox create pays. The pool is allowed to fill before the
//! burst starts, which is the state a host is in when a create arrives.

// Sample counts and nanosecond totals are far below the ranges these conversions can lose, and a
// load driver reads best as one sequence.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

use std::{
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

use soma::{DnsPolicy, EgressPolicy, NetworkPolicy};
use soma_netd::{
    BrokerClient, ControlAuthority, InstanceId, NetworkIntent, OperationId, Reply, Request,
    broker_owner, serve,
};

use super::{control::current_group, session};

fn percentile(sorted: &[u128], numerator: usize) -> u128 {
    sorted[(sorted.len() * numerator / 100).min(sorted.len() - 1)]
}

/// Runs `rounds` bursts of `concurrency` clients against a daemon holding `pool` bundles.
pub fn burst(concurrency: usize, rounds: usize, pool: usize) {
    // The broker keeps its state under /run, which is a tmpfs, so the ledger lives there too.
    let state = tempfile::tempdir_in("/dev/shm").expect("state dir");
    let socket_dir = tempfile::tempdir().expect("socket dir");
    // The control socket directory must belong to the broker identity and be private.
    std::fs::set_permissions(
        socket_dir.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o750),
    )
    .expect("socket dir mode");
    let socket = socket_dir.path().join("broker.sock");
    let broker = super::broker(state.path(), 4096);
    let intent = NetworkIntent::admit(
        &NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
            .expect("policy"),
        &broker.profile().clone(),
    )
    .expect("intent");
    let authority = ControlAuthority::new(
        broker_owner(),
        current_group(),
        &[broker_owner()],
        &[broker_owner()],
    )
    .expect("authority");
    let path = socket.clone();
    thread::spawn(move || {
        let _ = serve(broker, &path, pool, authority);
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline && !socket.exists() {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(socket.exists(), "the broker never bound its socket");
    // Let the workers fill the pool: that is the steady state a create arrives in.
    let settle: u64 = std::env::var("SOMA_NETD_LOAD_SETTLE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    thread::sleep(Duration::from_secs(settle));

    let mut claim = Vec::new();
    let mut activate = Vec::new();
    let mut release = Vec::new();
    let mut total = Vec::new();
    for round in 0..rounds {
        let barrier = Arc::new(Barrier::new(concurrency));
        // Sandboxes live a while after they are created; their release is a later, separate
        // burst, so it cannot pollute the create path that is being measured.
        let teardown = Arc::new(Barrier::new(concurrency));
        let handles: Vec<_> = (0..concurrency)
            .map(|index| {
                let barrier = Arc::clone(&barrier);
                let teardown = Arc::clone(&teardown);
                let socket = socket.clone();
                let intent = intent.clone();
                thread::spawn(move || {
                    let mut bytes = [0_u8; 16];
                    bytes[0] = 0x7a;
                    bytes[1..5].copy_from_slice(&(round as u32).to_be_bytes());
                    bytes[5..9].copy_from_slice(&(index as u32).to_be_bytes());
                    bytes[15] = 1;
                    let instance = InstanceId::new(bytes).expect("instance");
                    let operation = OperationId::new(bytes).expect("operation");
                    let host = session::repaired(bytes, bytes);
                    // With `SOMA_NETD_LOAD_RECONNECT` every call opens its own connection, which
                    // is the only way a daemon that serves one connection at a time can be
                    // measured at all.
                    let reconnect = std::env::var_os("SOMA_NETD_LOAD_RECONNECT").is_some();
                    let connect = || BrokerClient::connect(&socket).expect("connect");
                    // Connected up front only when the connection is kept, because a daemon that
                    // serves one connection at a time would otherwise wait on every idle one.
                    let mut client = if reconnect { None } else { Some(connect()) };
                    barrier.wait();
                    let start = Instant::now();
                    if reconnect {
                        client = Some(connect());
                    }
                    let (reply, _tap) = client
                        .as_ref()
                        .expect("connection")
                        .call(&Request::Claim {
                            instance,
                            operation,
                            vsock_cid: 3 + index as u32,
                            intent: intent.clone(),
                        })
                        .expect("claim");
                    let claimed = start.elapsed();
                    let Reply::Claimed {
                        bundle,
                        generation,
                        activation,
                        ..
                    } = reply
                    else {
                        panic!("claim refused: {reply:?}");
                    };
                    let receipt = host
                        .network_activation(&activation, generation.get(), intent.digest().0)
                        .expect("receipt");
                    let start_activate = Instant::now();
                    if reconnect {
                        client = Some(connect());
                    }
                    let (reply, _) = client
                        .as_ref()
                        .expect("connection")
                        .call(&Request::Activate {
                            bundle,
                            generation,
                            receipt,
                        })
                        .expect("activate");
                    assert!(matches!(reply, Reply::Activated), "{reply:?}");
                    let activated = start_activate.elapsed();
                    if reconnect {
                        // Nothing may hold a connection open across the wait: a daemon that
                        // serves one connection at a time would block on it for its whole idle
                        // timeout.
                        client = None;
                    }
                    teardown.wait();
                    thread::sleep(Duration::from_millis(500));
                    let start_release = Instant::now();
                    if reconnect {
                        client = Some(connect());
                    }
                    let (reply, _) = client
                        .as_ref()
                        .expect("connection")
                        .call(&Request::Release { bundle, generation })
                        .expect("release");
                    assert!(
                        matches!(reply, Reply::Released { complete: true }),
                        "{reply:?}"
                    );
                    (claimed, activated, start_release.elapsed())
                })
            })
            .collect();
        for handle in handles {
            let (c, a, r) = handle.join().expect("client");
            claim.push(c.as_nanos());
            activate.push(a.as_nanos());
            release.push(r.as_nanos());
            total.push((c + a).as_nanos());
        }
        // The next burst starts from a refilled pool, as a host between creates would.
        thread::sleep(Duration::from_secs(settle));
    }
    for (name, values) in [
        ("claim", &mut claim),
        ("activate", &mut activate),
        ("claim_plus_activate", &mut total),
        ("release", &mut release),
    ] {
        values.sort_unstable();
        println!(
            "load c={concurrency} pool={pool} op={name} n={} min_ms={:.2} p50_ms={:.2} p90_ms={:.2} p99_ms={:.2} max_ms={:.2}",
            values.len(),
            values[0] as f64 / 1e6,
            percentile(values, 50) as f64 / 1e6,
            percentile(values, 90) as f64 / 1e6,
            percentile(values, 99) as f64 / 1e6,
            values[values.len() - 1] as f64 / 1e6,
        );
    }
}
