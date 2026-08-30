//! The one compiled Generation and the one captured snapshot every test in this file shares.
//!
//! Compiling a real `node:22` Generation and booting it costs minutes, so it happens once per
//! test process and every test borrows the result. The capture itself is the proof of the
//! first half of the ticket, so it is asserted here rather than in one test.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock, PoisonError},
    time::{Duration, Instant},
};

use soma_generation::open_artifact;
use soma_guest::ResponderKeypair;
use soma_kvm::x86_64::{
    CaptureOutcome, CaptureRequest, Milestone, SandboxEvidence, SandboxMachine, SnapshotPaths,
    capture,
};

use crate::{
    x86_64_discover::kernel_path, x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::scratch_dir, x86_64_sandbox_boot_session as session,
};

/// The image the captured Generation is built from.
pub const IMAGE: &str = "node:22";
/// Environment variable naming a pre-exported OCI layout for it.
pub const LAYOUT_VAR: &str = "SOMA_OCI_NODE_LAYOUT";
/// Guest RAM of the captured machine, matching the cold-boot evidence for `node:22`.
pub const MEMORY_MIB: u64 = 1024;
/// Writable class of the captured machine; every restore clones a head of this size.
pub const STORAGE_MIB: u64 = 256;
/// The context identifier the captured machine holds; every restore is assigned another.
pub const CAPTURE_CID: u32 = 3;
/// The exact console line the pinned guest agent prints at the disconnected repair point.
pub const REPAIR_POINT_LINE: &[u8] = b"soma-guest-agent: awaiting launch material";
/// How long the guest may take to reach the repair point.
pub const REPAIR_POINT_DEADLINE: Duration = Duration::from_secs(120);
/// How long vCPU 0 may take to leave `KVM_RUN` after the capture kicks it.
pub const PAUSE_GRACE: Duration = Duration::from_secs(10);

const MIB: u64 = 1024 * 1024;

/// Everything the tests share, built once.
pub struct Fixture {
    pub scratch: PathBuf,
    pub paths: SnapshotPaths,
    pub capture: CaptureOutcome,
    pub compiled: generation::Compiled,
    pub responder_public: [u8; 32],
    pub responder_private: [u8; 32],
    pub generation_id: [u8; 32],
    pub ram_bytes: u64,
    /// The pinned static guest agent the Generation was built with.
    pub agent: PathBuf,
    /// The evidence of the machine the snapshot was taken from.
    pub source: SandboxEvidence,
}

impl Fixture {
    /// A fresh read-only handle on the immutable Generation root.
    pub fn root(&self) -> File {
        let manifest = &self.compiled.generation.published.manifest;
        open_artifact(&self.compiled.store, &manifest.root.descriptor).expect("open the root")
    }

    /// Clones one Instance-private overlay head from the snapshot's sterile template.
    pub fn private_head(&self, name: &str) -> (PathBuf, File) {
        let directory = self.scratch.join("heads");
        fs::create_dir_all(&directory).expect("create the head directory");
        let path = directory.join(format!("{name}.ext4"));
        let _ignored = fs::remove_file(&path);
        fs::copy(self.paths.overlay(), &path).expect("clone the sterile template");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("open the private head");
        (path, file)
    }
}

/// The shared fixture as every test borrows it.
pub type Shared = MutexGuard<'static, Fixture>;

static FIXTURE: OnceLock<Option<Mutex<Fixture>>> = OnceLock::new();

/// Prints the one reason a test may decline to run and returns.
pub fn skip() {
    eprintln!("SKIP: the node:22 OCI layout could not be exported; set {LAYOUT_VAR}");
}

/// Builds the shared fixture on first use and lends it to every later caller.
///
/// Returns `None` when the image cannot be exported, which is a skip rather than a failure.
pub fn shared() -> Option<Shared> {
    // The build result is recorded once, success or skip, so a suite that cannot export the
    // image declines in seconds instead of recompiling the Generation for every test.
    let built = FIXTURE.get_or_init(|| build().map(Mutex::new));
    Some(
        built
            .as_ref()?
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    )
}

fn build() -> Option<Fixture> {
    let scratch = scratch_dir("node22");
    let inputs = generation::inputs(kernel_path());
    let layout = generation::oci_layout(IMAGE, LAYOUT_VAR, &scratch)?;
    let responder = ResponderKeypair::generate().expect("responder keypair");
    let responder_private = session::responder_key(&responder);
    let compiled = generation::compile(
        &layout,
        &format!("docker.io/library/{IMAGE}"),
        generation::Shape {
            memory_mib: MEMORY_MIB,
            storage_mib: STORAGE_MIB,
        },
        &responder_private,
        &inputs,
        &scratch,
    );
    let manifest = &compiled.generation.published.manifest;
    let generation_id = session::generation_bytes(compiled.generation.id().as_str());
    eprintln!(
        "[capture] generation_id={} root={} ({} bytes) overlay_template={} ({} bytes) initramfs={}",
        compiled.generation.id().as_str(),
        manifest.root.descriptor.digest,
        manifest.root.descriptor.size,
        manifest.overlay.templates[0].descriptor.digest,
        manifest.overlay.templates[0].descriptor.size,
        manifest.initramfs.descriptor.digest,
    );

    let directory = scratch.join("snapshot");
    let _ignored = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create the snapshot directory");
    let paths = SnapshotPaths::new(directory);
    let ram_bytes = MEMORY_MIB * MIB;
    let (source, capture) = capture_source(&compiled, &paths, generation_id, ram_bytes, &scratch);
    Some(Fixture {
        scratch,
        paths,
        capture,
        compiled,
        responder_public: responder.public_key().to_bytes(),
        responder_private,
        generation_id,
        ram_bytes,
        agent: inputs.agent.clone(),
        source,
    })
}

/// Boots the Generation with no launch page at all, waits for the repair point, and captures.
fn capture_source(
    compiled: &generation::Compiled,
    paths: &SnapshotPaths,
    generation_id: [u8; 32],
    ram_bytes: u64,
    scratch: &Path,
) -> (SandboxEvidence, CaptureOutcome) {
    let manifest = &compiled.generation.published.manifest;
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
        ram_bytes,
    );
    let mut sandbox = SandboxMachine::create(config).expect("create the source machine");
    sandbox.watch_console(REPAIR_POINT_LINE);
    // No launch page is written: the machine must reach its repair point with no Instance
    // identity, no session, and no key anywhere in guest memory.
    sandbox.start().expect("start the source machine");
    let started = Instant::now();
    let outcome = capture(
        &mut sandbox,
        CaptureRequest {
            paths: paths.clone(),
            generation_id,
            root: &mut root,
            overlay: &mut head,
            repair_point_line: REPAIR_POINT_LINE.to_vec(),
            grace: PAUSE_GRACE,
        },
        started + REPAIR_POINT_DEADLINE,
    );
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
