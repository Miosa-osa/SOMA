//! The image-shaped live proofs: a tiny busybox machine, the node:22 machine, and the large
//! shape's own base image with the guest tuning the plan asks for.
//!
//! Split from the harness root so each file stays within the repository's source ceiling.

use std::path::Path;

use crate::{
    live::{BUSYBOX, boot_generation, serialize_live_proof},
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{assert_proof, require_kvm},
    x86_64_sandbox_boot_session as session,
};

const NODE: &str = "node:22";
/// The tree digest the node:22 export had when this proof was written.
const MAC_NODE_TREE_DIGEST: &str =
    "sha256:5dac6c571b970375a978c3f2f8777883e5bdd582fb4b43a5b872f929a2c7adf6";

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker"]
fn tiny_generation_boots_authenticates_and_executes_one_command() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/busybox",
        arguments: &[b"uname", b"-a"],
        timeout_millis: 10_000,
        output_bytes: 65_536,
    };
    let proof = boot_generation(
        "busybox",
        BUSYBOX,
        "SOMA_OCI_BUSYBOX_LAYOUT",
        generation::Shape::new(256, 64, 1),
        &command,
    )
    .expect("prerequisite failed: the busybox OCI layout could not be exported; install Docker or set SOMA_OCI_BUSYBOX_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    assert!(stdout.starts_with("Linux soma-"), "stdout={stdout:?}");
    assert!(stdout.contains("6.12.107-soma-v1"));
    assert!(stdout.contains("x86_64"));
    assert!(proof.executed.stderr.is_empty());
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker with node:22"]
fn node_22_generation_boots_authenticates_and_reports_its_version() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/usr/local/bin/node",
        arguments: &[b"--version"],
        timeout_millis: 30_000,
        output_bytes: 65_536,
    };
    let proof = boot_generation(
        "node22",
        NODE,
        "SOMA_OCI_NODE_LAYOUT",
        generation::Shape::new(1024, 1024, 1),
        &command,
    )
        .expect("prerequisite failed: the node:22 OCI layout could not be exported; set SOMA_OCI_NODE_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    assert!(stdout.starts_with("v22."), "stdout={stdout:?}");
    let _ = Path::new(MAC_NODE_TREE_DIGEST);
}

/// The image the large shape runs: Ubuntu 24.04 with everything the DAX script's `prepare`
/// installs, so a sandbox does not need the network just to become runnable.
const DAX_IMAGE: &str = "dax-base:24.04";
/// Environment variable naming a pre-exported OCI layout for it.
const DAX_LAYOUT_VAR: &str = "SOMA_OCI_DAX_LAYOUT";
/// Guest RAM of the large shape, in MiB.
const DAX_MEMORY_MIB: u64 = 16 * 1024;
/// Writable class of the large shape, in MiB: the DAX workload clones a repository and installs
/// a dependency tree into it, so the scratch has to be sized for a build rather than a test.
const DAX_STORAGE_MIB: u64 = 40 * 1024;

/// What the guest reports about itself: the shape, the filesystem, and the tuning.
///
/// Every line is a claim the plan makes. The writeback figures are absolute byte counts, so they
/// only say 16 GiB / 4 and 3/4 of that if the tuning actually ran on this machine's memory.
const DAX_REPORT: &str = "\
set -e
printf 'processors=%s\\n' \"$(nproc)\"
printf 'mem_total_mib=%s\\n' \"$(awk '/^MemTotal:/ {print int($2 / 1024)}' /proc/meminfo)\"
printf 'root_total_kib=%s\\n' \"$(df -k / | awk 'NR == 2 {print $2}')\"
printf 'root_free_kib=%s\\n' \"$(df -k / | awk 'NR == 2 {print $4}')\"
df -h /
free -g
printf 'root_options=%s\\n' \"$(findmnt -no OPTIONS /)\"
printf 'upper_options=%s\\n' \"$(findmnt -no OPTIONS /mnt/upper)\"
printf 'dirty_bytes=%s\\n' \"$(cat /proc/sys/vm/dirty_bytes)\"
printf 'dirty_background_bytes=%s\\n' \"$(cat /proc/sys/vm/dirty_background_bytes)\"
printf 'dirty_ratio=%s\\n' \"$(cat /proc/sys/vm/dirty_ratio)\"
printf 'swap_total_kib=%s\\n' \"$(awk '/^SwapTotal:/ {print $2}' /proc/meminfo)\"
";

/// Reads one `key=value` line the report printed.
fn reported(stdout: &str, key: &str) -> Option<u64> {
    let prefix = format!("{key}=");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.trim().parse().ok())
}

/// The large shape's gate: the DAX base image boots at eight processors, sixteen gigabytes, and
/// forty gigabytes of writable storage, with the plan's guest tuning already applied.
///
/// The tuning is asserted from two directions, because either alone proves less: the guest reads
/// the values back out of `/proc`, and the agent's own console line is required to appear before
/// the repair point, which is what makes the tuning part of the machine a capture records rather
/// than something a restored sandbox would have to do again.
#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and the dax-base OCI layout"]
fn the_dax_base_image_boots_tuned_in_the_large_shape() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/bash",
        arguments: &[b"-c", DAX_REPORT.as_bytes()],
        // Forty gigabytes of writable storage is created and copied on the host before this
        // command runs, so the command itself is quick; the bound is generous because a first
        // run also populates the page cache for a root above a gigabyte.
        timeout_millis: 120_000,
        output_bytes: 65_536,
    };
    let proof = boot_generation(
        "dax-base",
        DAX_IMAGE,
        DAX_LAYOUT_VAR,
        generation::Shape::new(DAX_MEMORY_MIB, DAX_STORAGE_MIB, 8),
        &command,
    )
    .expect("prerequisite failed: the dax-base OCI layout could not be exported; set SOMA_OCI_DAX_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    eprintln!("[dax-base] the guest reported:\n{stdout}");

    assert_eq!(
        reported(&stdout, "processors"),
        Some(8),
        "stdout={stdout:?}"
    );
    let total_mib = reported(&stdout, "mem_total_mib").expect("no memory total");
    assert!(
        total_mib >= 16_000,
        "the large shape was told about {total_mib} MiB, not sixteen gigabytes"
    );
    // The composed root is the writable class: a 40 GiB ext4 head, composed under the read-only
    // image, and `df` is the only place the guest can show it.
    let root_kib = reported(&stdout, "root_total_kib").expect("no root size");
    assert!(
        root_kib >= 39 * 1024 * 1024,
        "the writable root is {root_kib} KiB, not the forty gigabytes the shape declares"
    );
    // D1: the byte-capped writeback. The numbers are a quarter of sixteen gigabytes and three
    // quarters of that, so they can only be right if the tuning read this machine's memory.
    assert_eq!(
        reported(&stdout, "dirty_bytes"),
        Some(4 * 1024 * 1024 * 1024),
        "the writeback cap is not a quarter of this machine's RAM: stdout={stdout:?}"
    );
    assert_eq!(
        reported(&stdout, "dirty_background_bytes"),
        Some(3 * 1024 * 1024 * 1024),
        "the background threshold is not three quarters of the cap: stdout={stdout:?}"
    );
    assert_eq!(
        reported(&stdout, "dirty_ratio"),
        Some(0),
        "a non-zero ratio would let the kernel ignore the byte cap"
    );
    // D2: the scratch and the composed root are mounted without access-time updates.
    let root_options = stdout
        .lines()
        .find_map(|line| line.strip_prefix("root_options="))
        .unwrap_or_default();
    assert!(
        root_options.contains("noatime"),
        "the composed root is mounted with {root_options:?}"
    );
    let upper_options = stdout
        .lines()
        .find_map(|line| line.strip_prefix("upper_options="))
        .unwrap_or_default();
    assert!(
        upper_options.contains("noatime") && upper_options.contains("lazytime"),
        "the writable head is mounted with {upper_options:?}"
    );
    // D3: zram needs a kernel with swap and an image with the tools; whichever the guest found,
    // the agent reported it rather than failing the boot. The pinned kernel is built without
    // swap, so what this asserts today is the honest report.
    let serial = String::from_utf8_lossy(&proof.evidence.serial);
    let tuning_line = serial
        .lines()
        .find(|line| line.contains("tuned dirty_bytes="))
        .unwrap_or_else(|| panic!("the agent reported no tuning line"));
    eprintln!("[dax-base] {tuning_line}");
    assert!(
        tuning_line.contains("zram=enabled:") || tuning_line.contains("zram=no-kernel-device"),
        "the tuning line does not account for zram: {tuning_line:?}"
    );
    // The tuning has to be part of the machine a capture records, not something a restored
    // sandbox applies afterwards, so its line comes before the repair point the capture uses.
    let repair_point = serial
        .find("awaiting launch material")
        .expect("the guest never reached its repair point");
    let tuned_at = serial.find("tuned dirty_bytes=").expect("no tuning line");
    assert!(
        tuned_at < repair_point,
        "the tuning was applied after the repair point, so a capture would not carry it"
    );
}
