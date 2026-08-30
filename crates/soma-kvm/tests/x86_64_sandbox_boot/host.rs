//! Host-side helpers for the live sandbox proof: prerequisites, scratch space, descriptor and
//! thread accounting, and the assertions every successful run must satisfy.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, SystemTime},
};

use soma_guest::TerminalStatus;
use soma_kvm::x86_64::{LAUNCH_PAGE_GPA, Milestone, SandboxEvidence};

use crate::x86_64_sandbox_boot_session as session;

pub fn require_kvm() {
    let ok = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok();
    assert!(
        ok,
        "prerequisite failed: this live test needs a readable and writable /dev/kvm; it never passes silently"
    );
}

pub fn open_descriptor_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .expect("the KVM live-test host must mount procfs")
        .count()
}

pub fn thread_count() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Threads:"))
                .and_then(|rest| rest.trim().parse().ok())
        })
        .unwrap_or(0)
}

/// How long a scratch tree may go untouched before the next run reclaims it.
///
/// A live run finishes in well under an hour, so anything older than this belongs to a run
/// that is over and its gigabytes are free to take back.
const SCRATCH_LIFETIME: Duration = Duration::from_hours(6);

/// Scratch space for this run, named for the caller and private to this process.
///
/// Cargo hands one `CARGO_TARGET_TMPDIR` to every profile, so a fixed name lets two live runs
/// of one worktree write the same snapshot objects and clone the same overlay heads, and each
/// then measures a tree the other is rewriting. The run token below makes every run's tree its
/// own; trees older than [`SCRATCH_LIFETIME`] are reclaimed so the directory stays bounded.
pub fn scratch_dir(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("x86_64-sandbox-boot");
    fs::create_dir_all(&root).expect("create the scratch root under target/");
    reclaim_stale(&root);
    require_free_space(&root);
    let dir = root.join(format!("{name}-{}", run_token()));
    fs::create_dir_all(&dir).expect("create scratch directory under target/");
    dir
}

/// Free space one run needs before it starts.
///
/// A `node:22` run writes an EROFS root near 1.1 GiB, an overlay head, and a snapshot, and
/// several runs share the scratch root within [`SCRATCH_LIFETIME`]. This is a floor, not a
/// measurement of one run.
const REQUIRED_FREE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Fails the run before it starts when the scratch filesystem cannot hold it.
///
/// Exhausted scratch space surfaces from deep inside Generation compilation as an opaque
/// toolchain failure, because the root formatter simply cannot write, and a partly written
/// tree can also strand a boot until its deadline. Both read as flaky live tests. Naming the
/// real condition here keeps that misreading from costing another investigation.
#[allow(unsafe_code)]
fn require_free_space(root: &Path) {
    let Ok(path) = std::ffi::CString::new(root.as_os_str().as_encoded_bytes()) else {
        return;
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a live null-terminated string and `stats` is a live, correctly aligned
    // `statvfs` that the call either fills or leaves untouched, which the result distinguishes.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return;
    }
    // SAFETY: `statvfs` returned zero, so it filled the structure.
    let stats = unsafe { stats.assume_init() };
    let available = stats.f_bavail.saturating_mul(stats.f_frsize);
    assert!(
        available >= REQUIRED_FREE_BYTES,
        "scratch filesystem at {} has {available} bytes free, below the {REQUIRED_FREE_BYTES} \
         one run needs; reclaim that directory, whose trees are kept for {} hours",
        root.display(),
        SCRATCH_LIFETIME.as_secs() / 3600,
    );
}

/// One identifier for this test process, stable for its whole life.
///
/// The process identifier alone is not enough: live runs happen in containers with their own
/// process namespaces, where two runs can hold the same number at the same time.
fn run_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        let since_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        format!("{}-{}", std::process::id(), since_epoch.as_nanos())
    })
}

/// Removes every scratch tree that has not been touched within [`SCRATCH_LIFETIME`].
fn reclaim_stale(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| {
                modified
                    .elapsed()
                    .is_ok_and(|since| since > SCRATCH_LIFETIME)
            });
        if stale {
            let _ignored = fs::remove_dir_all(entry.path());
        }
    }
}

/// Everything one successful run must leave behind.
pub struct Proof {
    pub evidence: SandboxEvidence,
    pub hostile: session::Executed,
    pub executed: session::Executed,
    pub fd_before: usize,
    pub fd_after: usize,
    pub threads_before: u64,
    pub threads_after: u64,
    pub root_before: String,
    pub root_after: String,
    pub head_before: String,
    pub head_after: String,
}

pub fn assert_proof(proof: &Proof) {
    session::assert_orderly(&proof.evidence);
    // The hostile step ran first: PID 1 bounded it at the exact allowance, killed its process
    // group, and then accepted the next lifecycle operation on the same authenticated session.
    assert_eq!(proof.hostile.status, TerminalStatus::OutputLimit);
    let hostile_bytes = proof.hostile.stdout.len() + proof.hostile.stderr.len();
    assert_eq!(
        u64::try_from(hostile_bytes).unwrap(),
        session::HOSTILE_ALLOWANCE,
        "the hostile step must deliver exactly its allowance"
    );
    // Whichever pipe the kernel makes readable first may legitimately spend the whole
    // allowance: the reader shares the remaining room only among the streams a single poll
    // pass finds ready, so a stream that has not been written yet cannot reserve any of it.
    // The live contract is the bound and the accounting, both asserted above. Deterministic
    // fairness, where both pipes are ready together, is asserted by the guest-agent unit test
    // `hostile_output_on_both_pipes_stays_within_a_declared_resident_bound`.
    assert!(
        !proof.hostile.stdout.is_empty() || !proof.hostile.stderr.is_empty(),
        "the hostile step delivered nothing on either pipe"
    );
    assert_eq!(proof.executed.status, TerminalStatus::Exited(0));
    assert_eq!(
        proof.fd_after, proof.fd_before,
        "the sandbox leaked descriptors"
    );
    assert_eq!(
        proof.threads_after, proof.threads_before,
        "the sandbox leaked threads"
    );
    assert_eq!(
        proof.root_before, proof.root_after,
        "the EROFS root changed"
    );
    assert_ne!(
        proof.head_before, proof.head_after,
        "the overlay head never changed"
    );
    assert_eq!(LAUNCH_PAGE_GPA, soma_guest::LAUNCH_PAGE_GUEST_ADDRESS);
    assert_eq!(soma_kvm::SOMA_CONTROL_PORT, soma_guest::CONTROL_VSOCK_PORT);
    let ready = proof.evidence.at(Milestone::Ready).unwrap();
    let start = proof.evidence.at(Milestone::RunStart).unwrap();
    assert!(ready > start);
}

#[cfg(test)]
mod tests {
    use super::{SCRATCH_LIFETIME, run_token, scratch_dir};
    use std::{
        fs,
        time::{Duration, SystemTime},
    };

    /// How old the abandoned tree in the test below is made: a week, chosen independently of
    /// the constant under test so a longer lifetime cannot be aged past silently.
    const A_WEEK: Duration = Duration::from_hours(168);

    /// Two live runs of one worktree must not share one tree, and an abandoned tree must go.
    #[test]
    fn a_scratch_tree_belongs_to_one_run_and_an_abandoned_one_is_reclaimed() {
        let mine = scratch_dir("isolation-probe");
        let again = scratch_dir("isolation-probe");
        assert_eq!(mine, again, "the token must be stable for one process");
        let name = mine.file_name().unwrap().to_str().unwrap().to_owned();
        assert_eq!(name, format!("isolation-probe-{}", run_token()));
        assert!(
            name.contains(&std::process::id().to_string()),
            "another run of this worktree would take the same tree: {name}"
        );

        let abandoned = mine.with_file_name(format!("isolation-abandoned-{}", run_token()));
        fs::create_dir_all(abandoned.join("snapshot")).expect("create the abandoned tree");
        assert!(
            SCRATCH_LIFETIME < A_WEEK,
            "a tree abandoned a week ago must be older than the lifetime"
        );
        let aged = SystemTime::now() - A_WEEK;
        fs::File::open(&abandoned)
            .expect("open the abandoned tree")
            .set_times(fs::FileTimes::new().set_modified(aged))
            .expect("age the abandoned tree");

        let _reclaims = scratch_dir("isolation-probe");

        assert!(!abandoned.exists(), "an abandoned tree kept its gigabytes");
        assert!(mine.exists(), "this run's own tree was reclaimed");
    }
}
