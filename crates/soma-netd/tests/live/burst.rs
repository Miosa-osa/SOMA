//! The hundred-way prepare, assign, activate, and release burst and its raw samples.

use soma::{DnsPolicy, EgressPolicy, NetworkPolicy};
use soma_netd::{Disposition, NetNamespace, NetworkIntent, activate, reconcile, release};

use super::{broker, session};

/// Runs one hundred complete bundle lifecycles and prints every raw sample.
pub fn hundred_way() {
    use std::time::Instant;

    let state = tempfile::tempdir().expect("state dir");
    let mut broker = broker(state.path(), 128);
    let intent = NetworkIntent::admit(
        &NetworkPolicy::new(EgressPolicy::PublicInternet, DnsPolicy::System, Vec::new())
            .expect("policy"),
        &broker.profile().clone(),
    )
    .expect("intent");
    let mut samples: [Vec<u128>; 4] = Default::default();
    for index in 0..100_u8 {
        let mut bytes = [0; 16];
        bytes[0] = 0xc0;
        bytes[1] = index;
        bytes[15] = 1;
        let bundle = soma_netd::BundleId::new(bytes).expect("bundle");
        let instance = soma_netd::InstanceId::new(bytes).expect("instance");
        let operation = soma_netd::OperationId::new(bytes).expect("operation");
        let start = Instant::now();
        let sterile = broker.prepare(bundle).expect("prepare");
        samples[0].push(start.elapsed().as_nanos());
        let start = Instant::now();
        let mut assigned = broker
            .assign(sterile, instance, operation, &intent, 3 + u32::from(index))
            .map_err(|failure| failure.error)
            .expect("assign");
        samples[1].push(start.elapsed().as_nanos());
        let host = session::repaired(*instance.as_bytes(), *operation.as_bytes());
        let receipt = session::mint(&host, &assigned);
        let start = Instant::now();
        activate(&mut assigned, &receipt).expect("activate");
        samples[2].push(start.elapsed().as_nanos());
        let start = Instant::now();
        let evidence = release(&broker, assigned);
        samples[3].push(start.elapsed().as_nanos());
        assert!(evidence.complete, "bundle {index} incomplete: {evidence:?}");
    }
    for (name, values) in ["prepare", "assign", "activate", "release"]
        .iter()
        .zip(samples.iter_mut())
    {
        values.sort_unstable();
        let p50 = values[values.len() / 2];
        let p99 = values[values.len() * 99 / 100];
        println!(
            "burst op={name} n={} min_ns={} p50_ns={p50} p99_ns={p99} max_ns={}",
            values.len(),
            values[0],
            values[values.len() - 1]
        );
        println!("burst raw op={name} ns={values:?}");
    }
    let report = reconcile(&broker).expect("reconcile");
    assert_eq!(report.entries.len(), 100);
    assert!(
        report
            .entries
            .iter()
            .all(|(_, _, d)| *d == Disposition::Released)
    );
    assert_eq!(report.unowned(), 0);
    assert!(
        NetNamespace::list(broker.namespace_dir())
            .expect("pins")
            .is_empty()
    );
}
