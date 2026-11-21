//! Restore captured execution after the original KVM and its worker have exited.

use std::{fs::File, io::Seek as _, time::Duration};

use soma_guest::{HostLaunchMaterial, LaunchNetwork, TerminalStatus};
use soma_kvm::{
    snapshot::Digest,
    x86_64::{
        CheckpointRequest, CheckpointTargets, Hypervisor, ResidentRestoreRequest, SandboxMachine,
        SnapshotObjects, capture_checkpoint, prepare_checkpoint,
    },
};

#[path = "resident/fork.rs"]
mod fork;
pub use fork::verify as verify_fork;

use crate::{
    x86_64_sandbox_boot_session as session,
    x86_64_sandbox_boot_sparse::clone_head,
    x86_64_snapshot_restore_fixture::{self, Fixture},
    x86_64_snapshot_restore_instance::{self as instance, command},
    x86_64_snapshot_restore_workload::{Session, Workload, execute},
};

const NODE: &[u8] = b"/usr/local/bin/node";
const CHILD: &[u8] = br"
const marker = require('node:crypto').randomBytes(16).toString('hex');
let ticks = 0;
const sample = () => [process.pid, marker, ticks].join(' ');
setInterval(() => ticks++, 5);
process.on('SIGUSR2', () => { ticks = 1_000_000; });
require('node:net').createServer(c => c.end(sample())).listen('/tmp/resident-recovery.sock');
";
const START: &[u8] = br"
const fs = require('node:fs');
const child = require('node:child_process').spawn(process.execPath, ['-e', process.argv[1]], {
    detached: true, stdio: 'ignore'
});
fs.writeFileSync('/root/soma-recovery-proof', 'captured');
fs.mkdirSync('/run/soma', {recursive: true});
fs.writeFileSync('/run/soma/survivors', child.pid + '\n');
child.unref();
";
const PROBE: &[u8] = br"
let retries = 100;
function tryRead() {
    const client = require('node:net').connect('/tmp/resident-recovery.sock');
    client.on('data', data => process.stdout.write(data));
    client.on('error', error => {
        if (--retries > 0) setTimeout(tryRead, 10);
        else throw error;
    });
}
tryRead();
";

struct Capture<'a> {
    instance_id: [u8; 16],
    fixture: &'a Fixture,
}

struct Saved {
    state: File,
    memory: File,
    overlay: File,
    digest: Digest,
    original: Sample,
}

#[derive(Debug)]
struct Sample {
    pid: u32,
    marker: String,
    ticks: u64,
}

impl Sample {
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        let pid = parts.next()?.parse().ok()?;
        let marker = parts.next()?.to_owned();
        let ticks = parts.next()?.parse().ok()?;
        (marker.len() == 32 && parts.next().is_none()).then_some(Self { pid, marker, ticks })
    }
}

impl Workload for Capture<'_> {
    type Output = Saved;

    fn bind_instance(&mut self, instance_id: [u8; 16]) {
        self.instance_id = instance_id;
    }

    fn run<'a>(
        &mut self,
        machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Saved), String> {
        let (session, started) = execute(session, &command(NODE, &[b"-e", START, CHILD]))?;
        assert_eq!(started.status, TerminalStatus::Exited(0));
        let (session, output) = execute(session, &command(NODE, &[b"-e", PROBE]))?;
        assert_eq!(output.status, TerminalStatus::Exited(0));
        let output = String::from_utf8_lossy(&output.stdout);
        let original = Sample::parse(&output).expect("source process sample");

        let state = tempfile::tempfile().expect("state");
        let memory = tempfile::tempfile().expect("memory");
        let overlay = tempfile::tempfile().expect("overlay");
        let manifest = capture_checkpoint(
            machine,
            &CheckpointRequest {
                instance_id: self.instance_id,
                candidate_id: self.fixture.candidate_id,
                root_digest: self.fixture.capture.root_digest,
                contract: self.fixture.contract,
                grace: Duration::from_secs(2),
                targets: CheckpointTargets {
                    state: &state,
                    memory: &memory,
                    overlay: Some(&overlay),
                },
            },
        )
        .map_err(|error| format!("save resident Instance: {error}"))?;
        let digest = Digest::of(&manifest.encode());
        let (session, updated) = execute(
            session,
            &command(NODE, &[b"-e", b"require('node:fs').writeFileSync('/root/soma-recovery-proof', 'after-checkpoint')"]),
        )?;
        assert_eq!(updated.status, TerminalStatus::Exited(0));
        Ok((
            session,
            Saved {
                state,
                memory,
                overlay,
                digest,
                original,
            },
        ))
    }
}

pub fn verify(fixture: &Fixture) {
    use soma_kvm::x86_64::{GuestExit, Milestone};

    let source = instance::run_workload(
        fixture,
        "resident-recovery-source",
        4,
        Capture {
            instance_id: [0; 16],
            fixture,
        },
    );
    assert_eq!(source.evidence.exit, Ok(GuestExit::Reset));
    assert!(source.evidence.at(Milestone::Shutdown).is_some());
    // The only descriptors that survive source shutdown are the three checkpoint artifacts.
    assert_eq!(source.descriptors.1, source.descriptors.0 + 3);
    assert_eq!(source.threads.0, source.threads.1);
    let Saved {
        state,
        memory,
        mut overlay,
        digest,
        original,
    } = source.output;
    overlay.rewind().expect("rewind saved overlay");
    let head_path = fixture.scratch.join("resident-recovery-head.ext4");
    let head = clone_head(&overlay, &head_path);
    let request = ResidentRestoreRequest {
        objects: SnapshotObjects::adopt(state, memory, Some(overlay)),
        hypervisor: Hypervisor::Device,
        root: fixture.root(),
        head: Some(head),
        instance_id: source.identity,
        candidate_id: fixture.candidate_id,
        state_digest: digest,
        devices: fixture.devices(),
        guest_cid: 5,
        memory_bytes: fixture.ram_bytes,
        vcpus: fixture.vcpus,
        contract: fixture.contract,
        hotplug: None,
    };
    let prepared = prepare_checkpoint(request).expect("rebuild stopped Instance");
    let operation = session::random16();
    let network = reconnect_network(5);
    let material =
        HostLaunchMaterial::generate(fixture.candidate_id, source.identity, operation, network)
            .expect("fresh reconnect material");
    let mut restored = None;
    let delivered = material
        .deliver_with(|page| {
            restored = Some(prepared.resume(page)?);
            Ok::<(), soma_kvm::x86_64::SnapshotError>(())
        })
        .expect("resume with fresh control authority");
    let restored = restored.expect("restored VM");
    let mut probe = crate::x86_64_snapshot_restore_workload::Commands(&[
        command(NODE, &[b"-e", PROBE]),
        command(b"/bin/cat", &[b"/root/soma-recovery-proof"]),
    ]);
    let outcome = instance::drive(
        &restored,
        delivered,
        &instance::Identity {
            instance: source.identity,
            operation,
        },
        &mut probe,
    );
    let ready = restored.is_ready();
    let report = restored.machine.finish(Duration::from_secs(2));
    std::fs::remove_file(head_path).expect("remove private restore head");
    let console = String::from_utf8_lossy(&report.serial);
    let (checks, transcript) = outcome.unwrap_or_else(|error| {
        panic!(
            "reconnect failed: {error}; exit={:?}; console={console:?}",
            report.exit
        )
    });
    assert!(ready);
    assert_ne!(source.session_transcript, transcript);
    assert_eq!(checks[0].status, TerminalStatus::Exited(0));
    assert_eq!(checks[1].status, TerminalStatus::Exited(0));
    assert_eq!(checks[1].stdout, b"captured");
    let sample = String::from_utf8_lossy(&checks[0].stdout);
    let after = Sample::parse(&sample).expect("authenticated process sample");
    assert_eq!(after.pid, original.pid);
    assert_eq!(after.marker, original.marker);
    assert!(after.ticks > original.ticks);
    assert_eq!(report.exit, Ok(GuestExit::Reset));
    eprintln!(
        "[resident-recovery] source shut down and the reconstructed guest reconnected with a fresh session, retained its process and marker, and recovered its earlier disk bytes, counter {} -> {}",
        original.ticks, after.ticks,
    );
}

fn reconnect_network(cid: u32) -> LaunchNetwork {
    LaunchNetwork::new(
        cid,
        cid,
        x86_64_snapshot_restore_fixture::launch_mac([0; 6]),
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        session::now_unix_nanos(),
    )
    .expect("disconnected reconnect network")
}
