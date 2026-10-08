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
use soma_kvm::MachineContract;
use soma_kvm::x86_64::{
    CaptureOutcome, CaptureRequest, Milestone, SandboxEvidence, SandboxMachine, SnapshotPaths,
    capture,
};

use crate::{
    x86_64_discover::kernel_path,
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{require_scratch_space, require_scratch_space_for, scratch_dir},
    x86_64_sandbox_boot_session as session,
};

const MIB: u64 = 1024 * 1024;

mod recipe;

pub use recipe::{
    BUSYBOX_V2, CAPTURE_CID, LARGE_V2, NODE22, PAUSE_GRACE, REPAIR_POINT_DEADLINE,
    REPAIR_POINT_LINE, Recipe, STORAGE_MIB, V2_REQUIRED_FREE_BYTES,
};

/// The MAC the launch page carries: the captured one, or the fixed
/// link-down placeholder when the Generation declared no network device
/// (the capture then holds the zero MAC, which the page refuses).
pub fn launch_mac(captured: [u8; 6]) -> [u8; 6] {
    if captured == [0; 6] {
        [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01]
    } else {
        captured
    }
}

/// Everything the tests share, built once.
pub struct Fixture {
    pub scratch: PathBuf,
    pub paths: SnapshotPaths,
    pub capture: CaptureOutcome,
    pub compiled: generation::Compiled,
    pub candidate_id: [u8; 32],
    pub ram_bytes: u64,
    /// vCPUs the captured machine ran, which every restore must come back with.
    pub vcpus: u16,
    /// The contract the Generation was compiled and captured under.
    pub contract: MachineContract,
    /// The pinned static guest agent the Generation was built with.
    pub agent: PathBuf,
    /// The evidence of the machine the snapshot was taken from.
    pub source: SandboxEvidence,
}

impl Fixture {
    /// The device set the Generation declares - the one the snapshot was
    /// captured with. Every restore must name this same set: the machine
    /// contract digest binds it, so a test pinning its own `DeviceSet` is a
    /// different machine and an Incompatible(MachineContract) refusal.
    pub fn devices(&self) -> soma_kvm::DeviceSet {
        self.compiled.manifest().device_set()
    }

    /// A fresh read-only handle on the immutable Generation root.
    pub fn root(&self) -> File {
        let manifest = &self.compiled.manifest();
        open_artifact(&self.compiled.store, &manifest.root.descriptor).expect("open the root")
    }

    /// The capacity every private head cloned from this snapshot's template has.
    ///
    /// A prepared worker builds its overlay slot against this before it may hold a head.
    pub fn overlay_capacity_bytes(&self) -> u64 {
        fs::metadata(self.paths.overlay())
            .expect("stat the sterile overlay template")
            .len()
    }

    /// Clones one Instance-private overlay head from the snapshot's sterile template.
    pub fn private_head(&self, name: &str) -> (PathBuf, File) {
        let directory = self.scratch.join("heads");
        fs::create_dir_all(&directory).expect("create the head directory");
        let path = directory.join(format!("{name}.ext4"));
        // Cloned the way the boot harness clones one: the class is written out whole, so a copy
        // that does not skip its zero chunks costs a whole class of disk per head.
        let template = File::open(self.paths.overlay()).expect("open the sterile template");
        let file = crate::x86_64_sandbox_boot_sparse::clone_head(&template, &path);
        (path, file)
    }
}

/// The shared fixture as every test borrows it.
pub type Shared = MutexGuard<'static, Fixture>;

static NODE22_FIXTURE: OnceLock<Mutex<Fixture>> = OnceLock::new();
static BUSYBOX_V2_FIXTURE: OnceLock<Mutex<Fixture>> = OnceLock::new();

/// Builds the shared fixture on first use and lends it to every later caller.
///
/// # Panics
///
/// Panics when the `node:22` image cannot be exported. These tests are `#[ignore]`d, so a run
/// that reaches here asked for them by name or with `--ignored`; a missing prerequisite is
/// then a failed run and never a test that reports `ok` having executed nothing.
pub fn shared() -> Shared {
    borrow(&NODE22_FIXTURE, &NODE22)
}

/// The shared eight-vCPU, sixteen-gigabyte fixture the contract v2 proofs borrow.
pub fn shared_v2() -> Shared {
    borrow(&BUSYBOX_V2_FIXTURE, &BUSYBOX_V2)
}

/// Compiles one recipe's Generation and captures one snapshot of it, once per process.
///
/// The Generation is compiled once and every later caller borrows it, so a suite that cannot
/// export the image fails in seconds instead of recompiling it for every test.
pub(crate) fn borrow(cell: &'static OnceLock<Mutex<Fixture>>, recipe: &Recipe) -> Shared {
    cell.get_or_init(|| Mutex::new(build(recipe)))
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn build(recipe: &Recipe) -> Fixture {
    // The capture walk writes the whole memory object, so a multi-vCPU shape stages its own
    // figure rather than the floor a small machine fits in.
    if recipe.vcpus > 1 {
        require_scratch_space_for(V2_REQUIRED_FREE_BYTES);
    } else {
        require_scratch_space();
    }
    let scratch = scratch_dir(recipe.scratch);
    let inputs = generation::inputs(kernel_path());
    let layout = generation::oci_layout(recipe.image, recipe.layout_var, &scratch).unwrap_or_else(
        || {
            panic!(
                "prerequisite failed: the {} OCI layout could not be exported; set {}. It never passes silently",
                recipe.image, recipe.layout_var
            )
        },
    );
    let compiled = generation::compile(
        &layout,
        &format!("docker.io/library/{}", recipe.image),
        generation::Shape {
            memory_mib: recipe.memory_mib,
            storage_mib: recipe.storage_mib,
            vcpus: recipe.vcpus,
        },
        &inputs,
        &scratch,
    );
    let manifest = &compiled.manifest();
    let candidate_id = session::generation_bytes(compiled.id().as_str());
    eprintln!(
        "[capture] candidate_id={} root={} ({} bytes) overlay_template={} ({} bytes) initramfs={}",
        compiled.id().as_str(),
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
    let ram_bytes = recipe.memory_mib * MIB;
    let (source, capture) =
        capture_source(&compiled, &paths, candidate_id, ram_bytes, &scratch, recipe);
    Fixture {
        scratch,
        paths,
        capture,
        compiled,
        candidate_id,
        ram_bytes,
        vcpus: recipe.vcpus,
        contract: recipe.contract,
        agent: inputs.agent.clone(),
        source,
    }
}

/// Boots the Generation with no launch page at all, waits for the repair point, and captures.
fn capture_source(
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
            grace: PAUSE_GRACE,
            contract: recipe.contract,
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
