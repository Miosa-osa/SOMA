//! Capture the boot snapshot a fixture restores from.

use super::{
    CaptureOutcome, CaptureRequest, Duration, Instant, Milestone, PAUSE_GRACE, Path,
    REPAIR_POINT_DEADLINE, REPAIR_POINT_LINE, Recipe, SandboxEvidence, SandboxMachine,
    SnapshotPaths, capture, fs, generation, open_artifact, session,
};

/// Boots the Generation with no launch page at all, waits for the repair point, and captures.
pub(super) fn capture_source(
    compiled: &generation::Compiled,
    paths: &SnapshotPaths,
    candidate_id: [u8; 32],
    ram_bytes: u64,
    scratch: &Path,
    recipe: &Recipe,
) -> (SandboxEvidence, CaptureOutcome) {
    let manifest = &compiled.manifest();
    let kernel = open_artifact(&compiled.store, &manifest.kernel.descriptor).unwrap();
    let initramfs = open_artifact(&compiled.store, &manifest.initramfs.descriptor).unwrap();
    let mut root = open_artifact(&compiled.store, &manifest.root.descriptor).unwrap();
    let mut template =
        open_artifact(&compiled.store, &manifest.overlay.templates[0].descriptor).unwrap();
    let head_path = scratch.join("capture-head.ext4");
    let mut head = generation::private_head(&mut template, &head_path);
    drop(template);

    let config = session::config(
        kernel,
        initramfs,
        open_artifact(&compiled.store, &manifest.root.descriptor).unwrap(),
        head.try_clone().unwrap(),
        session::Machine {
            ram_bytes,
            vcpus: recipe.vcpus,
            contract: recipe.contract,
        },
        manifest.device_set(),
    );
    let mut sandbox = SandboxMachine::create(config).expect("create the source machine");
    assert!(
        sandbox.pause(PAUSE_GRACE).is_err(),
        "a prepared guest cannot be paused"
    );
    sandbox.watch_console(REPAIR_POINT_LINE);
    // No launch page is written: the machine must reach its repair point with no Instance
    // identity, no session, and no key anywhere in guest memory.
    sandbox.start().expect("start the source machine");
    let started = Instant::now();
    let outcome = capture(
        &mut sandbox,
        CaptureRequest {
            paths: paths.clone(),
            candidate_id,
            root: &mut root,
            overlay: Some(&mut head),
            repair_point_line: REPAIR_POINT_LINE.to_vec(),
            point: soma_kvm::x86_64::CapturePoint::ConsoleLine,
            grace: PAUSE_GRACE,
            contract: recipe.contract,
        },
        started + REPAIR_POINT_DEADLINE,
    );
    if outcome.is_ok() {
        assert!(sandbox.is_paused(), "capture must leave the source paused");
        sandbox
            .pause(PAUSE_GRACE)
            .expect("replay the completed pause");
        assert!(sandbox.is_paused(), "replay must keep the machine paused");
    }
    let evidence = sandbox.finish(Duration::from_secs(10));
    let log = scratch.join("capture-serial.log");
    fs::write(&log, &evidence.serial).unwrap();
    let console = String::from_utf8_lossy(&evidence.serial);
    let announced = console.contains(&String::from_utf8_lossy(REPAIR_POINT_LINE).into_owned());
    eprintln!(
        "[capture] console {} bytes retained at {}; repair point announced on it: {announced}",
        evidence.serial.len(),
        log.display()
    );
    if outcome.is_err() {
        for line in console
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .iter()
            .rev()
        {
            eprintln!("  | {line}");
        }
    }
    let outcome = outcome.expect("capture the machine at the repair point");
    assert!(
        evidence.at(Milestone::RunStart).is_some(),
        "the source machine never entered KVM_RUN"
    );
    eprintln!(
        "[capture] posted receive buffers at the capture point: net={} vsock={} events={}",
        outcome.posted_buffers[0], outcome.posted_buffers[1], outcome.posted_buffers[2],
    );
    eprintln!(
        "[capture] memory={} ({} bytes) overlay={} ({} bytes) state={} ({} bytes) root={}",
        outcome.memory_digest,
        outcome.memory_bytes,
        outcome.overlay_digest,
        outcome.overlay_bytes,
        outcome.state_digest,
        outcome.state_bytes,
        outcome.root_digest,
    );
    (evidence, outcome)
}
