//! The large shape's capture proof: the tuned DAX Generation is captured at the repair point and
//! restored into a live Instance that still carries its tuning.
//!
//! This is what makes the tuning machine state rather than a script: the settings are applied by
//! the guest agent before the repair point, the capture records the machine as it stands there,
//! and a restored Instance reads them back without running anything. The busybox proof next door
//! covers the shape; this one covers the shape the workload runs in, with the image that carries
//! the workload's own runtime.

use std::sync::{Mutex, OnceLock};

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_sandbox_boot_session as session,
    x86_64_snapshot_restore_fixture::{self as fixture, Fixture, LARGE_V2, Shared},
    x86_64_snapshot_restore_instance as instance,
};

/// The one captured large machine every test in this file borrows.
static LARGE_FIXTURE: OnceLock<Mutex<Fixture>> = OnceLock::new();

fn shared() -> Shared {
    fixture::borrow(&LARGE_FIXTURE, &LARGE_V2)
}

/// What the restored guest reports: the tuning it was captured with, and the runtime it carries.
const REPORT: &str = r#"set -u
echo "NPROC=$(nproc)"
echo "DIRTY_BYTES=$(cat /proc/sys/vm/dirty_bytes)"
echo "DIRTY_BACKGROUND_BYTES=$(cat /proc/sys/vm/dirty_background_bytes)"
echo "DIRTY_RATIO=$(cat /proc/sys/vm/dirty_ratio)"
echo "SWAP_DEVICES=$(awk 'NR>1 {print $1}' /proc/swaps | tr '\n' ' ')"
echo "CHRT1=$(chrt -p 1 2>&1 | tr '\n' ' ')"
echo "NODE=$(node --version)"
echo "PROC_VERSION_BYTES=$(wc -c < /proc/version)"
echo "HOSTS_LOCALHOST=$(grep -c localhost /etc/hosts)"
echo "END""#;

/// Captures the large Generation and restores it, requiring the tuning to have come back with it.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and the large OCI layout"]
fn the_large_generation_captures_and_restores_carrying_its_tuning() {
    require_kvm();
    let fixture = shared();
    assert_eq!(fixture.vcpus, 8, "the large recipe is not the eight-vCPU shape");
    let command = session::Command {
        program: b"/bin/bash",
        arguments: &[b"-c", REPORT.as_bytes()],
        timeout_millis: 120_000,
        output_bytes: 1 << 16,
    };
    let restored = instance::run(&fixture, "large", 9, &[command]);
    let stdout = String::from_utf8_lossy(&restored.output[0].stdout);
    eprintln!("[large] the restored guest reported:\n{stdout}");

    // The tuning is the claim: a restored Instance that had to re-tune itself would report the
    // kernel's own defaults here, and an Instance that lost the swap device would report none.
    assert_eq!(
        reported(&stdout, "dirty_bytes"),
        Some(4 * 1024 * 1024 * 1024),
        "the restored Instance did not carry its writeback cap: stdout={stdout:?}"
    );
    assert_eq!(
        reported(&stdout, "dirty_background_bytes"),
        Some(3 * 1024 * 1024 * 1024),
        "stdout={stdout:?}"
    );
    assert_eq!(reported(&stdout, "dirty_ratio"), Some(0), "stdout={stdout:?}");
    assert!(
        stdout.contains("SWAP_DEVICES=/dev/zram0"),
        "the restored Instance lost its swap device: stdout={stdout:?}"
    );
    assert!(
        stdout.contains("SCHED_FIFO"),
        "the restored Instance lost its realtime policy: stdout={stdout:?}"
    );
    assert_eq!(reported(&stdout, "nproc"), Some(8), "stdout={stdout:?}");
    assert!(
        stdout.contains("NODE=v22."),
        "the restored Instance cannot run the workload's runtime: stdout={stdout:?}"
    );
    assert_eq!(
        reported(&stdout, "proc_version_bytes"),
        Some(0),
        "the restored Instance exposed its kernel version: stdout={stdout:?}"
    );
    assert!(stdout.contains("END"), "the guest script did not finish");
}

/// Reads one `key=value` line as its value, ignoring the key's case.
fn line<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        name.eq_ignore_ascii_case(key).then_some(value.trim())
    })
}

/// Reads one `key=value` line as a number.
fn reported(stdout: &str, key: &str) -> Option<u64> {
    line(stdout, key)?.parse().ok()
}
