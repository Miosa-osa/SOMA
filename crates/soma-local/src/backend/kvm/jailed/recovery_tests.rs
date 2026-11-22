use std::path::PathBuf;

use soma::{InstanceId, OperationId};
use soma_guest::{GuestCommand, TerminalStatus};
use soma_kvm::{MachineContract, snapshot::Digest};
use soma_vmm::sandbox::CheckpointSource;

use crate::backend::kvm::{
    checkpoint,
    held::Held,
    identity::{fresh16, generation_bytes, instance_bytes},
    prepared,
};

use super::{Anchors, Jailed, Launching};

const START: &[u8] = br"
const fs = require('node:fs');
const script = `
const marker = require('node:crypto').randomBytes(16).toString('hex');
let counter = 0;
setInterval(() => counter++, 5);
process.on('SIGUSR2', () => { counter = 1_000_000; });
require('node:net').createServer(socket => {
    socket.end([process.pid, marker, counter].join(' '));
}).listen('/tmp/restore-rehearsal.sock');
`;
const child = require('node:child_process').spawn(process.execPath, ['-e', script], {
    detached: true, stdio: 'ignore'
});
fs.writeFileSync('/root/restore-rehearsal', 'saved');
fs.mkdirSync('/run/soma', {recursive: true});
fs.writeFileSync('/run/soma/survivors', child.pid + '\n');
child.unref();
";
const SAMPLE: &[u8] = br"
let remaining = 100;
function sample() {
    const socket = require('node:net').connect('/tmp/restore-rehearsal.sock');
    socket.on('data', bytes => process.stdout.write(bytes));
    socket.on('error', error => {
        if (--remaining) setTimeout(sample, 10);
        else throw error;
    });
}
sample();
";

fn execute(jailed: &mut Jailed, program: &[u8], args: &[&[u8]]) -> Vec<u8> {
    let command = GuestCommand::new(
        program.to_vec(),
        args.iter().map(|arg| arg.to_vec()).collect(),
        5000,
        4096,
    )
    .expect("bounded command");
    let result = jailed
        .execute(&command)
        .expect("authenticated sealed command");
    assert_eq!(
        result.status,
        TerminalStatus::Exited(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
}

fn check_filters(jailed: &Jailed) {
    let pid = jailed.handle.as_ref().expect("live jail").pid();
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("worker status");
    assert!(status.lines().any(|line| line == "Seccomp:\t2"));
    assert!(status.lines().any(|line| line == "Seccomp_filters:\t2"));
}

#[test]
#[ignore = "requires the Linux KVM worker, a private delegated cgroup, and a prepared Node 22 Generation with the reconnect agent"]
fn a_sealed_worker_recovers_a_published_checkpoint_after_reaping_the_source() {
    let anchors = Anchors::configured().expect("isolated test jail");
    let root = PathBuf::from(std::env::var_os("SOMA_GENERATION_STORE").expect("prepared store"));
    let prepared = prepared::find(Some(&root), "node:22").expect("verified prepared Generation");
    let hex = |bytes| format!("{:032x}", u128::from_be_bytes(bytes));
    let instance = InstanceId::new(hex(fresh16())).expect("Instance");
    let checkpoint_id = OperationId::new(hex(fresh16())).expect("checkpoint");
    let mut launching = Launching {
        operation: &checkpoint_id,
        prepared: &prepared,
        instance: &instance,
        instance_bytes: instance_bytes(&instance).expect("identity bytes"),
        generation_bytes: generation_bytes(&prepared.id).expect("Generation bytes"),
        memory_mib: prepared.manifest.shape.memory_bytes / (1024 * 1024),
        disk_mib: prepared.manifest.template.writable_storage_bytes / (1024 * 1024),
        checkpoint: None,
        volume: None,
    };
    let mut original =
        Jailed::launch(&anchors, &launching, None).expect("launch original in its jail");
    check_filters(&original);
    let pid = original.handle.as_ref().unwrap().pid();
    let node = b"/usr/local/bin/node";
    execute(&mut original, node, &[b"-e", START]);
    let before = execute(&mut original, node, &[b"-e", SAMPLE]);
    let mut held = Held::Jailed(Box::new(original));
    let receipt = checkpoint::perform(
        &mut held,
        &instance,
        &prepared.id,
        &checkpoint_id,
        CheckpointSource {
            instance_id: launching.instance_bytes,
            candidate_id: launching.generation_bytes,
            root_digest: Digest::from_bytes(*prepared.manifest.root.descriptor.digest.as_bytes()),
            contract: MachineContract::require(prepared.manifest.machine_contract.version).unwrap(),
            has_overlay: true,
            has_volume: false,
        },
        false,
    )
    .expect("publish checkpoint from the sealed worker");
    let Held::Jailed(mut original) = held else {
        unreachable!("the original is jailed")
    };
    execute(
        &mut original,
        node,
        &[
            b"-e",
            b"require('node:fs').writeFileSync('/root/restore-rehearsal', 'later')",
        ],
    );
    let cleanup = original.release(false).expect("clean shutdown");
    assert_eq!(cleanup, soma::CleanupMethod::Graceful);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());

    launching.checkpoint = Some((&instance, &checkpoint_id));
    let mut recovered =
        Jailed::launch(&anchors, &launching, None).expect("recover in a fresh jail");
    check_filters(&recovered);
    let recovered_pid = recovered.handle.as_ref().unwrap().pid();
    assert_ne!(pid, recovered_pid);
    let after = execute(&mut recovered, node, &[b"-e", SAMPLE]);
    let sample = |bytes: &[u8]| {
        let text = std::str::from_utf8(bytes).expect("guest process sample");
        let mut words = text.split_ascii_whitespace();
        (
            words.next().unwrap().to_owned(),
            words.next().unwrap().to_owned(),
            words.next().unwrap().parse::<u64>().expect("counter"),
        )
    };
    let (pid_before, marker_before, ticks_before) = sample(&before);
    let (pid_after, marker_after, ticks_after) = sample(&after);
    assert_eq!(pid_before, pid_after);
    assert_eq!(marker_before, marker_after);
    assert!(ticks_after > ticks_before);
    assert_eq!(
        execute(&mut recovered, b"/bin/cat", &[b"/root/restore-rehearsal"]),
        b"saved"
    );
    assert_eq!(
        recovered.release(false).expect("reap recovered guest"),
        soma::CleanupMethod::Graceful
    );
    assert!(!std::path::Path::new(&format!("/proc/{recovered_pid}")).exists());
    eprintln!(
        "[sealed-recovery] Generation={} state={} worker {} -> {}, original guest PID and marker retained, counter {} -> {}, saved disk read and both jails reaped",
        receipt.generation_id.as_str(),
        receipt.state_digest,
        pid,
        recovered_pid,
        ticks_before,
        ticks_after,
    );
}

mod fork;
mod workspace;
