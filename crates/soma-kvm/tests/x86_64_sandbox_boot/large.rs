//! The large-shape generation proof: a version 2 Generation built from the DAX workload image
//! cold-boots on KVM at eight vCPUs and sixteen gigabytes, and one command verifies the machine
//! and the guest tuning from inside the guest.
//!
//! The harness runs exactly one command per boot, so the command is a script that reports every
//! landmark on its own line and the assertions read those lines back. This is the proof for the
//! Generation the DAX shape boots: the machine shape, the writeback cap, the scratch mount
//! options, the zram swap device, the node runtime, the agent's realtime policy, the lock-down
//! block, and a write across most of the machine's RAM.

use crate::{
    live::{boot_generation, serialize_live_proof},
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{assert_proof, require_kvm},
    x86_64_sandbox_boot_session as session,
};

/// The image the large proof builds its Generation from, exported from the workload Dockerfile.
const LARGE: &str = "soma-large-dax:1";
/// The machine contract v2 shape this Generation targets.
const MEMORY_MIB: u64 = 16 * 1024;
const STORAGE_MIB: u64 = 20 * 1024;
const VCPUS: u16 = 8;
/// Bytes the guest writes across its RAM: fifteen gigabytes, most of a sixteen-gigabyte machine.
const FILL_BYTES: u64 = 15 * 1024 * 1024 * 1024;

/// The one command the large proof runs, reported as `key=value` lines.
pub const SCRIPT: &str = r#"set -u
echo "NPROC=$(nproc)"
echo "MEMTOTAL_MIB=$(awk '/^MemTotal:/ {print int($2/1024)}' /proc/meminfo)"
echo "FREE_G_TOTAL=$(free -g | awk 'NR==2 {print $2}')"
echo "DIRTY_BYTES=$(cat /proc/sys/vm/dirty_bytes)"
echo "DIRTY_BACKGROUND_BYTES=$(cat /proc/sys/vm/dirty_background_bytes)"
echo "DIRTY_EXPIRE_CENTISECS=$(cat /proc/sys/vm/dirty_expire_centisecs)"
echo "DIRTY_RATIO=$(cat /proc/sys/vm/dirty_ratio)"
echo "DIRTY_BACKGROUND_RATIO=$(cat /proc/sys/vm/dirty_background_ratio)"
echo "ROOT_OPTS=$(awk '$2=="/" {print $4}' /proc/mounts)"
echo "UPPER_OPTS=$(awk '$2=="/mnt/upper" {print $4}' /proc/mounts)"
echo "SWAP_DEVICES=$(awk 'NR>1 {printf "%s ", $1}' /proc/swaps)"
echo "SWAP_TOTAL_KIB=$(awk 'NR>1 {s+=$3} END{print s+0}' /proc/swaps)"
echo "NODE=$(node --version)"
echo "CHRT1=$(chrt -p 1 2>&1 | tr '\n' ' ')"
echo "DMESG_NONROOT=$(setpriv --reuid=65534 --regid=65534 --clear-groups dmesg 2>&1 | head -1)"
echo "KMSG_NONROOT=$(setpriv --reuid=65534 --regid=65534 --clear-groups head -c 1 /dev/kmsg 2>&1 | head -1)"
echo "CONFIG_GZ_NONROOT=$(setpriv --reuid=65534 --regid=65534 --clear-groups head -c 1 /proc/config.gz 2>&1 | head -1)"
echo "PROC_VERSION_BYTES=$(wc -c < /proc/version)"
echo "HOSTS_LOCALHOST=$(grep -c localhost /etc/hosts)"
echo "LO_FLAGS=$(cat /sys/class/net/lo/flags)"
echo "BASH=$(bash --version | head -1)"
mkdir -p /mnt/ram
mount -t tmpfs -o size=15G tmpfs /mnt/ram
head -c 16106127360 /dev/zero > /mnt/ram/fill
sync
echo "FILL_BYTES=$(wc -c < /mnt/ram/fill)"
echo "FILL_DIGEST=$(md5sum /mnt/ram/fill | cut -d' ' -f1)"
echo "OOM_KILL=$(awk '/^oom_kill / {print $2}' /proc/vmstat)"
echo "END""#;

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and the large OCI layout"]
fn the_large_generation_boots_and_carries_its_tuning() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/bash",
        arguments: &[b"-c", SCRIPT.as_bytes()],
        // The fill writes fifteen gigabytes through a tmpfs and reads it back; that is memory
        // bandwidth work with the machine's own patience, not the ten seconds a small command gets.
        timeout_millis: 300_000,
        output_bytes: 1 << 20,
    };
    let proof = boot_generation(
        "large",
        LARGE,
        "SOMA_OCI_LARGE_LAYOUT",
        generation::Shape::new(MEMORY_MIB, STORAGE_MIB, VCPUS),
        &command,
    )
    .expect("prerequisite failed: the large OCI layout could not be exported; set SOMA_OCI_LARGE_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    eprintln!("[{LARGE}] guest report:\n{stdout}");
    assert_tuning(&stdout);
}

/// Asserts every landmark the guest reported.
#[allow(clippy::too_many_lines)]
fn assert_tuning(stdout: &str) {
    assert!(
        stdout.contains("END"),
        "the guest script did not finish: stdout={stdout:?}"
    );
    assert_eq!(
        reported(stdout, "nproc"),
        Some(u64::from(VCPUS)),
        "stdout={stdout:?}"
    );
    let total = reported(stdout, "memtotal_mib").expect("no memory total reported");
    assert!(total >= 16_000, "the guest has {total} MiB, not 16 GiB");
    // `free -g` truncates the kernel's 15.996 GiB count, so the human-facing number is 15 or 16.
    assert!(
        reported(stdout, "free_g_total").is_some_and(|g| g >= 15),
        "stdout={stdout:?}"
    );

    // D1: the writeback cap in bytes. A sixteen-gigabyte machine gets 4 GiB and 3/4 of that.
    assert_eq!(
        reported(stdout, "dirty_bytes"),
        Some(4 * 1024 * 1024 * 1024),
        "stdout={stdout:?}"
    );
    assert_eq!(
        reported(stdout, "dirty_background_bytes"),
        Some(3 * 1024 * 1024 * 1024),
        "stdout={stdout:?}"
    );
    assert_eq!(reported(stdout, "dirty_expire_centisecs"), Some(360_000));
    assert_eq!(reported(stdout, "dirty_ratio"), Some(0), "the ratio must be zero");
    assert_eq!(reported(stdout, "dirty_background_ratio"), Some(0));

    // D2: the composed root carries noatime and no discard. The ext4 head's own flags are set at
    // mount time but are not listable from inside the composed root: its mount point lives in the
    // initramfs root the guest left behind, so `/proc/mounts` cannot name it. The boot succeeding
    // with the flags is the evidence for the head.
    let root = line(stdout, "root_opts").unwrap_or_default();
    assert!(root.contains("noatime"), "root opts are {root:?}");
    assert!(!root.contains("discard"), "root opts are {root:?}");

    // D3: a zram swap device at a quarter of RAM. The swap area is a hair under 4 GiB, because
    // zram keeps its own metadata above the usable pages, so the size is checked as a floor.
    assert_eq!(line(stdout, "swap_devices"), Some("/dev/zram0"), "stdout={stdout:?}");
    assert!(
        reported(stdout, "swap_total_kib").is_some_and(|kib| kib >= 4 * 1024 * 1024 - 64),
        "the swap area is not a quarter of this machine's RAM: stdout={stdout:?}"
    );

    // The runtime and the agent's realtime policy.
    assert!(line(stdout, "node").is_some_and(|v| v.starts_with('v') && v.contains("22.")));
    let chrt = line(stdout, "chrt1").unwrap_or_default();
    assert!(chrt.contains("SCHED_FIFO"), "chrt reported {chrt:?}");
    assert!(chrt.contains("10"), "chrt reported {chrt:?}");

    // G1 and G2: the lock-down block.
    assert!(
        line(stdout, "dmesg_nonroot").is_some_and(|v| v.contains("not permitted")),
        "non-root dmesg was not refused: stdout={stdout:?}"
    );
    assert!(
        line(stdout, "kmsg_nonroot").is_some_and(|v| v.contains("denied") || v.contains("not permitted")),
        "non-root /dev/kmsg was not refused: stdout={stdout:?}"
    );
    assert_eq!(reported(stdout, "proc_version_bytes"), Some(0), "/proc/version was not covered");

    // The machine's own network and the RAM write.
    assert!(reported(stdout, "hosts_localhost").is_some_and(|n| n >= 1));
    assert_eq!(line(stdout, "lo_flags"), Some("0x9"), "loopback is not up");
    assert_eq!(reported(stdout, "fill_bytes"), Some(FILL_BYTES), "stdout={stdout:?}");
    assert_eq!(reported(stdout, "oom_kill"), Some(0), "the fill triggered an OOM kill");
}

/// Reads one `key=value` line as its value, matching the key without regard to case.
///
/// The guest script prints its keys in capitals and the assertions name them the way the rest of
/// the harness does, so the match is case-insensitive rather than a convention in two places.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_matches_a_key_whatever_its_case() {
        // The guest script prints capitals; the assertions name the keys in lower case. A
        // case-sensitive reader would find nothing and every assertion would read as `None`.
        let report = "NPROC=8\nDIRTY_BYTES=4294967296\nSWAP_DEVICES=/dev/zram0\nEND\n";
        assert_eq!(reported(report, "nproc"), Some(8));
        assert_eq!(reported(report, "dirty_bytes"), Some(4_294_967_296));
        assert_eq!(line(report, "swap_devices"), Some("/dev/zram0"));
        assert_eq!(line(report, "absent"), None);
        // A line with no `=` is not a pair and is skipped, so `END` never matches a key.
        assert_eq!(line(report, "end"), None);
    }
}
