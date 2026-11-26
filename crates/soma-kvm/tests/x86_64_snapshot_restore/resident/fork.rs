use soma_guest::HostLaunchMaterial;
use soma_kvm::x86_64::{GuestExit, PreparedCheckpoint, SnapshotError, prepare_fork};
use std::{
    io::Seek as _,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use super::{
    Capture, Fixture, Hypervisor, NODE, PROBE, ResidentRestoreRequest, Sample, SandboxMachine,
    Saved, Session, SnapshotObjects, TerminalStatus, Workload, clone_head, command, execute,
    instance, reconnect_network, session,
};

const SYNC: Duration = Duration::from_secs(20);
const FILE: &[u8] = b"/root/soma-recovery-proof";

struct Observe {
    identity: [u8; 16],
    original_pid: u32,
    first: bool,
    send: Sender<()>,
    receive: Receiver<()>,
}

impl Workload for Observe {
    type Output = Sample;

    fn run<'a>(
        &mut self,
        _: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Sample), String> {
        let (session, machine_id) = execute(session, &command(b"/bin/cat", &[b"/etc/machine-id"]))?;
        assert_eq!(machine_id.status, TerminalStatus::Exited(0));
        let expected = format!("{:032x}", u128::from_be_bytes(self.identity));
        assert_eq!(
            String::from_utf8_lossy(&machine_id.stdout).trim_end(),
            expected
        );
        let (session, hostname) = execute(
            session,
            &command(b"/bin/cat", &[b"/proc/sys/kernel/hostname"]),
        )?;
        assert_eq!(hostname.status, TerminalStatus::Exited(0));
        assert_eq!(
            String::from_utf8_lossy(&hostname.stdout).trim_end(),
            format!("soma-{}", &expected[..12])
        );

        if self.first {
            let pid = self.original_pid.to_string();
            let (session, write) = execute(
                session,
                &command(NODE, &[b"-e", b"process.kill(Number(process.argv[1]), 'SIGUSR2'); require('node:fs').writeFileSync('/root/soma-recovery-proof','fork-a')", pid.as_bytes()]),
            )?;
            assert_eq!(write.status, TerminalStatus::Exited(0));
            self.send.send(()).expect("notify sibling");
            self.receive
                .recv_timeout(SYNC)
                .expect("sibling wrote independently");
            let (session, disk) = execute(session, &command(b"/bin/cat", &[FILE]))?;
            assert_eq!(disk.status, TerminalStatus::Exited(0));
            assert_eq!(disk.stdout, b"fork-a");
            sample(session)
        } else {
            self.receive.recv_timeout(SYNC).expect("first fork wrote");
            let (session, disk) = execute(session, &command(b"/bin/cat", &[FILE]))?;
            assert_eq!(disk.status, TerminalStatus::Exited(0));
            assert_eq!(disk.stdout, b"captured");
            let (session, write) = execute(
                session,
                &command(
                    NODE,
                    &[
                        b"-e",
                        b"require('node:fs').writeFileSync('/root/soma-recovery-proof','fork-b')",
                    ],
                ),
            )?;
            assert_eq!(write.status, TerminalStatus::Exited(0));
            self.send.send(()).expect("notify first fork");
            sample(session)
        }
    }
}

fn sample(session: Session<'_>) -> Result<(Session<'_>, Sample), String> {
    let (session, reply) = execute(session, &command(NODE, &[b"-e", PROBE]))?;
    assert_eq!(reply.status, TerminalStatus::Exited(0));
    let sample = Sample::parse(&String::from_utf8_lossy(&reply.stdout)).expect("resident process");
    Ok((session, sample))
}

struct Fork {
    prepared: PreparedCheckpoint,
    authority: HostLaunchMaterial,
    address: instance::Identity,
}

impl Fork {
    fn prepare(fixture: &Fixture, saved: &Saved, original: [u8; 16], cid: u32) -> Self {
        let mut overlay = saved.overlay.try_clone().unwrap();
        overlay.rewind().expect("rewind captured overlay");
        let head_path = fixture.scratch.join(format!("resident-fork-{cid}.ext4"));
        let head = clone_head(&overlay, &head_path);
        std::fs::remove_file(head_path).expect("unlink the fork's private writable head");
        let request = ResidentRestoreRequest {
            objects: SnapshotObjects::adopt(
                saved.state.try_clone().unwrap(),
                saved.memory.try_clone().unwrap(),
                Some(overlay),
            ),
            hypervisor: Hypervisor::Device,
            root: fixture.root(),
            head: Some(head),
            volume: None,
            instance_id: original,
            candidate_id: fixture.candidate_id,
            state_digest: saved.digest,
            devices: fixture.devices(),
            guest_cid: cid,
            memory_bytes: fixture.ram_bytes,
            vcpus: fixture.vcpus,
            contract: fixture.contract,
            hotplug: None,
            // Every fork after the first skips the memory read, as a broker's restore would.
            memory_attestation: soma_kvm::x86_64::attest_memory(
                &mut saved.memory.try_clone().unwrap(),
            )
            .expect("attest the captured memory"),
        };
        let address = instance::Identity {
            instance: session::random16(),
            operation: session::random16(),
        };
        let pending = prepare_fork(request, address.instance).expect("fork the same checkpoint");
        let material = HostLaunchMaterial::generate(
            fixture.candidate_id,
            address.instance,
            address.operation,
            reconnect_network(cid),
        )
        .expect("fresh destination session")
        .for_fork(original)
        .expect("authorized source");
        Self {
            prepared: pending,
            authority: material,
            address,
        }
    }

    fn observe(self, mut work: Observe) -> (Sample, [u8; 32]) {
        let Self {
            prepared,
            authority,
            address,
        } = self;
        let mut restored = None;
        let launch = authority
            .deliver_with(|page| {
                restored = Some(prepared.resume(page)?);
                Ok::<(), SnapshotError>(())
            })
            .expect("resume fork");
        let restored = restored.expect("live fork");
        let result = instance::drive(&restored, launch, &address, &mut work);
        let ready = restored.is_ready();
        let report = restored.machine.finish(Duration::from_secs(2));
        let checks = result.unwrap_or_else(|error| {
            panic!(
                "fork failed: {error}, exit={:?}, guest console={:?}",
                report.exit,
                String::from_utf8_lossy(&report.serial)
            )
        });
        assert!(ready);
        assert_eq!(report.exit, Ok(GuestExit::Reset));
        checks
    }
}

pub fn verify(fixture: &Fixture) {
    let source = instance::run_workload(
        fixture,
        "resident-fork-source",
        4,
        Capture {
            instance_id: [0; 16],
            fixture,
        },
    );
    assert_eq!(source.evidence.exit, Ok(GuestExit::Reset));
    let saved = source.output;
    let first = Fork::prepare(fixture, &saved, source.identity, 5);
    let second = Fork::prepare(fixture, &saved, source.identity, 6);
    assert_ne!(first.address.instance, second.address.instance);
    let (sent_a, seen_b) = mpsc::channel();
    let (sent_b, seen_a) = mpsc::channel();
    let work_a = Observe {
        identity: first.address.instance,
        original_pid: saved.original.pid,
        first: true,
        send: sent_a,
        receive: seen_a,
    };
    let work_b = Observe {
        identity: second.address.instance,
        original_pid: saved.original.pid,
        first: false,
        send: sent_b,
        receive: seen_b,
    };
    let ((a, transcript_a), (b, transcript_b)) = thread::scope(|scope| {
        let a = scope.spawn(move || first.observe(work_a));
        let b = second.observe(work_b);
        (a.join().expect("first fork"), b)
    });
    assert_ne!(transcript_a, transcript_b);
    assert_ne!(transcript_a, source.session_transcript);
    assert_ne!(transcript_b, source.session_transcript);
    assert_eq!(a.pid, saved.original.pid);
    assert_eq!(b.pid, saved.original.pid);
    assert_eq!(a.marker, saved.original.marker);
    assert_eq!(b.marker, saved.original.marker);
    assert!(a.ticks >= 1_000_000 && b.ticks < 1_000_000);
    eprintln!(
        "[resident-fork] two authenticated forks from one checkpoint, separate guest identities, private disk writes, private RAM counters {} and {}",
        a.ticks, b.ticks
    );
}
