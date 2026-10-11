//! A live resize of a cgroup leaf changes the CPU the processes in it actually get.
//!
//! Four busy loops run in one leaf. At a quota of one CPU they may use about one second of CPU
//! per second; after the quota is raised to three they use clearly more, and after it is lowered
//! again they are held back. The kernel's own accounting (`cpu.stat`) is the witness, not the
//! value read back from `cpu.max`.
//!
//! Needs a delegated cgroup2 subtree in `SOMA_JAIL_CGROUP_ROOT` and fails explicitly without it;
//! `scripts/jail-live-tests.sh` provides one.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use soma_jail::{CgroupLeaf, CpuMax};

const PERIOD_US: u64 = 100_000;
const BOOTED: u32 = 4;
const WINDOW: Duration = Duration::from_millis(1500);

fn usage_us(dir: &Path) -> u64 {
    fs::read_to_string(dir.join("cpu.stat"))
        .expect("cpu.stat")
        .lines()
        .find_map(|line| line.strip_prefix("usage_usec "))
        .expect("usage_usec")
        .trim()
        .parse()
        .expect("number")
}

/// CPU seconds the leaf consumed over one window, as a multiple of wall time.
fn cpus_used(dir: &Path) -> f64 {
    let before = usage_us(dir);
    let started = Instant::now();
    thread::sleep(WINDOW);
    let used = usage_us(dir) - before;
    #[allow(clippy::cast_precision_loss)]
    let ratio = used as f64 / (started.elapsed().as_micros() as f64);
    ratio
}

#[test]
#[ignore = "privileged live test; needs SOMA_JAIL_CGROUP_ROOT, see scripts/jail-live-tests.sh"]
fn a_live_cpu_resize_changes_the_cpu_the_leaf_gets() {
    let root = PathBuf::from(
        std::env::var_os("SOMA_JAIL_CGROUP_ROOT")
            .expect("prerequisite failed: SOMA_JAIL_CGROUP_ROOT (a delegated cgroup2 subtree)"),
    );
    // A host that was not started in a jail gets a leaf that limits CPU and nothing else; this is
    // the leaf a live resize writes.
    let leaf = CgroupLeaf::create_cpu_only(&root, "resize-live").expect("leaf");
    let dir = root.join("resize-live");
    let mut spinners: Vec<Child> = (0..BOOTED)
        .map(|_| {
            Command::new("sh")
                .args(["-c", "while :; do :; done"])
                .stdin(Stdio::null())
                .spawn()
                .expect("spinner")
        })
        .collect();
    for spinner in &spinners {
        leaf.attach(spinner.id()).expect("joined the leaf");
    }

    let share = |vcpus: u32| CpuMax {
        quota_us: u64::from(vcpus) * PERIOD_US,
        period_us: PERIOD_US,
    };
    leaf.set_cpu_max(share(1)).expect("one cpu");
    thread::sleep(Duration::from_millis(300));
    let one = cpus_used(&dir);
    leaf.set_cpu_max(share(3)).expect("three cpus");
    thread::sleep(Duration::from_millis(300));
    let three = cpus_used(&dir);
    leaf.set_cpu_max(share(1)).expect("back to one");
    thread::sleep(Duration::from_millis(300));
    let again = cpus_used(&dir);

    leaf.kill_all().expect("kill");
    for spinner in &mut spinners {
        let _ = spinner.wait();
    }
    leaf.remove(Instant::now() + Duration::from_secs(5))
        .expect("remove");

    println!("cpus used: quota 1 -> {one:.2}, quota 3 -> {three:.2}, quota 1 again -> {again:.2}");
    assert!(one <= 1.25, "a quota of one CPU was exceeded: {one:.2}");
    assert!(again <= 1.25, "lowering the quota did not hold: {again:.2}");
    assert!(
        three > one * 1.6,
        "raising the quota to three CPUs gave no more CPU: {three:.2} against {one:.2}"
    );
    assert!(
        three <= 3.25,
        "a quota of three CPUs was exceeded: {three:.2}"
    );
}
