use std::path::PathBuf;

use super::{
    Anchors, CheckpointSource, Digest, Held, InstanceId, Jailed, Launching, MachineContract,
    OperationId, SAMPLE, START, check_filters, checkpoint, execute, fresh16, generation_bytes,
    instance_bytes, prepared,
};

#[test]
#[ignore = "requires an isolated KVM host and a certified fork-capable Node 22 Generation"]
fn sealed_forks_keep_independent_identities_memory_and_disks() {
    let anchors = Anchors::configured().expect("isolated test jail");
    let root = PathBuf::from(std::env::var_os("SOMA_GENERATION_STORE").expect("prepared store"));
    let prepared = prepared::find(Some(&root), "node:22").expect("verified prepared Generation");
    let hex = |bytes| format!("{:032x}", u128::from_be_bytes(bytes));
    let source = InstanceId::new(hex(fresh16())).expect("source Instance");
    let a = InstanceId::new(hex(fresh16())).expect("first destination Instance");
    let b = InstanceId::new(hex(fresh16())).expect("second destination Instance");
    let checkpoint_id = OperationId::new(hex(fresh16())).expect("checkpoint operation");
    let launch = |instance: &InstanceId| {
        let origin = (instance != &source).then_some((&source, &checkpoint_id));
        Jailed::launch(
            &anchors,
            &Launching {
                prepared: &prepared,
                instance,
                instance_bytes: instance_bytes(instance).unwrap(),
                generation_bytes: generation_bytes(&prepared.id).unwrap(),
                memory_mib: prepared.manifest.shape.memory_bytes / (1024 * 1024),
                disk_mib: prepared.manifest.template.writable_storage_bytes / (1024 * 1024),
                checkpoint: origin,
                volume: None,
            },
            None,
        )
        .expect("launch the sealed worker")
    };
    let mut original = launch(&source);
    check_filters(&original);
    let node = b"/usr/local/bin/node";
    execute(&mut original, node, &[b"-e", START]);
    let parent_sample = sample_resident_process(&mut original);
    let mut held = Held::Jailed(Box::new(original));
    let receipt = checkpoint::perform(
        &mut held,
        &source,
        &prepared.id,
        &checkpoint_id,
        CheckpointSource {
            instance_id: instance_bytes(&source).unwrap(),
            candidate_id: generation_bytes(&prepared.id).unwrap(),
            root_digest: Digest::from_bytes(*prepared.manifest.root.descriptor.digest.as_bytes()),
            contract: MachineContract::require(prepared.manifest.machine_contract.version).unwrap(),
            has_overlay: true,
        },
        false,
    )
    .expect("publish isolated fixture checkpoint");
    let Held::Jailed(mut source_worker) = held else {
        unreachable!("source was jailed");
    };
    let mut worker_a = launch(&a);
    let mut worker_b = launch(&b);
    let worker_pids = verify_worker_identities([
        (&mut *source_worker, &source),
        (&mut worker_a, &a),
        (&mut worker_b, &b),
    ]);
    let write = |machine: &mut Jailed, value: &str, signal: bool| {
        let change = if signal {
            format!("process.kill({}, 'SIGUSR2');", parent_sample.0)
        } else {
            String::new()
        };
        let script = format!(
            "{change} require('node:fs').writeFileSync('/root/restore-rehearsal', '{value}')"
        );
        execute(machine, node, &[b"-e", script.as_bytes()]);
    };
    let file = |machine: &mut Jailed| execute(machine, b"/bin/cat", &[b"/root/restore-rehearsal"]);
    write(&mut worker_a, "branch-a", true);
    assert_eq!(file(&mut worker_b), b"saved");
    write(&mut worker_b, "branch-b", false);
    assert_eq!(file(&mut worker_a), b"branch-a");
    assert_eq!(file(&mut worker_b), b"branch-b");
    assert_eq!(file(&mut source_worker), b"saved");
    let a = sample_resident_process(&mut worker_a);
    let b = sample_resident_process(&mut worker_b);
    let source = sample_resident_process(&mut source_worker);
    for fork in [&a, &b, &source] {
        assert_eq!(fork.0, parent_sample.0);
        assert_eq!(fork.1, parent_sample.1);
    }
    assert!(a.2 >= 1_000_000 && b.2 < 1_000_000 && source.2 < 1_000_000);
    for machine in [*source_worker, worker_a, worker_b] {
        assert_eq!(
            machine.release(false).unwrap(),
            soma::CleanupMethod::Graceful
        );
    }
    for pid in &worker_pids {
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
    eprintln!(
        "[sealed-fork] Generation={} checkpoint={} three independent sealed workers; counters source={} A={} B={}; separate writable heads and clean shutdowns",
        receipt.generation_id.as_str(),
        receipt.state_digest,
        source.2,
        a.2,
        b.2,
    );
}

fn verify_worker_identities(workers: [(&mut Jailed, &InstanceId); 3]) -> Vec<i32> {
    let mut worker_pids = Vec::new();
    for (machine, expected) in workers {
        check_filters(machine);
        worker_pids.push(machine.handle.as_ref().unwrap().pid());
        let id = execute(machine, b"/bin/cat", &[b"/etc/machine-id"]);
        assert_eq!(
            std::str::from_utf8(&id).unwrap().trim_end(),
            expected.as_str()
        );
    }
    worker_pids.sort_unstable();
    worker_pids.dedup();
    assert_eq!(worker_pids.len(), 3);
    worker_pids
}

fn sample_resident_process(machine: &mut Jailed) -> (String, String, u64) {
    let output = execute(machine, b"/usr/local/bin/node", &[b"-e", SAMPLE]);
    let text = std::str::from_utf8(&output).unwrap();
    let mut words = text.split_ascii_whitespace();
    (
        words.next().unwrap().to_owned(),
        words.next().unwrap().to_owned(),
        words.next().unwrap().parse().unwrap(),
    )
}
