//! What a guest runs to write across all sixteen gigabytes, and what its report must say.
//!
//! Shared by the cold-boot memory proof and the snapshot proof, because a machine contract v2
//! machine has to be able to write all of its RAM whether the memory it is writing to was just
//! mapped or came back out of a snapshot.

/// Guest RAM: the sixteen gigabytes machine contract v2 admits, in MiB.
pub const MEMORY_MIB: u64 = 16 * 1024;
/// What the guest fills: most of that RAM, reaching well past the reserved MMIO hole, in MiB.
pub const FILL_MIB: u64 = 14 * 1024;
/// The exact byte count that fill writes, which the guest must read back.
pub const FILL_BYTES: u64 = FILL_MIB * 1024 * 1024;
/// The digest of that many zero bytes, so the read back proves the bytes and not just the size.
pub const FILL_DIGEST: &str = "9bacd7682ac03e217826a432d97138eb";

/// What the guest runs: report the machine, fill a tmpfs with most of the RAM, read it all back.
///
/// The tmpfs is the shortest path to memory the machine can measure: every page of it is guest
/// RAM resident until the mount goes away. The digest is a real pass over the bytes, not a stat,
/// so a page the machine cannot map fails here rather than becoming a number that looks right.
pub const SCRIPT: &str = "\
set -e
printf \"processors=%s\\n\" \"$(nproc)\"
printf \"mem_total_mib=%s\\n\" \"$(awk '/^MemTotal:/ {print int($2 / 1024)}' /proc/meminfo)\"
mkdir -p /mnt/ram
mount -t tmpfs -o size=15G tmpfs /mnt/ram
head -c 15032385536 /dev/zero > /mnt/ram/fill
sync
printf \"fill_bytes=%s\\n\" \"$(wc -c < /mnt/ram/fill)\"
printf \"fill_digest=%s\\n\" \"$(md5sum /mnt/ram/fill | cut -d' ' -f1)\"
printf \"mem_available_mib=%s\\n\" \"$(awk '/^MemAvailable:/ {print int($2 / 1024)}' /proc/meminfo)\"
printf \"tmpfs_used_kib=%s\\n\" \"$(df -k /mnt/ram | awk 'NR == 2 {print $3}')\"";

/// Reads one `key=value` line the script printed.
#[must_use]
pub fn reported(stdout: &str, key: &str) -> Option<u64> {
    let prefix = format!("{key}=");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.trim().parse().ok())
}

/// Asserts the guest's report: it has eight processors, and it wrote across all of its RAM.
///
/// Every clause is a different way for the machine to be wrong: fewer processors means the MP
/// table or a vCPU state section did not come back, a short read back means a page of the high
/// range is not backed, a digest that differs means the bytes are not the ones written, and
/// memory that stayed available means the fill was served from somewhere other than guest RAM.
pub fn assert_wrote_all_the_ram(stdout: &str) {
    assert_eq!(reported(stdout, "processors"), Some(8), "stdout={stdout:?}");
    let total = reported(stdout, "mem_total_mib").expect("the guest reported no memory total");
    assert!(
        total >= 16_000,
        "the guest was told about {total} MiB, not sixteen gigabytes: stdout={stdout:?}"
    );
    assert_eq!(
        reported(stdout, "fill_bytes"),
        Some(FILL_BYTES),
        "the guest did not read back every byte it wrote: stdout={stdout:?}"
    );
    assert!(
        stdout.contains(&format!("fill_digest={FILL_DIGEST}")),
        "the bytes read back are not the ones written: stdout={stdout:?}"
    );
    let used = reported(stdout, "tmpfs_used_kib").expect("the guest reported no tmpfs usage");
    assert!(
        used >= FILL_BYTES / 1024,
        "the tmpfs holds {used} KiB, less than the {FILL_MIB} MiB written"
    );
    let available = reported(stdout, "mem_available_mib").expect("no available memory reported");
    assert!(
        available < 2_000,
        "writing {FILL_MIB} MiB left {available} MiB available, so the fill did not consume guest memory"
    );
}
