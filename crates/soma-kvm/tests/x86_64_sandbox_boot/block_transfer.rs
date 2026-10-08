//! The block device's transfer limit and the guest's scratch filesystem, proven from inside a
//! live large-shape guest.
//!
//! Two host-side caps used to force a workaround in the guest: a block request above a mebibyte
//! was dropped by the walker instead of answered, so `sync` after any larger write never
//! returned, and `/tmp` was a 64 MiB tmpfs, so a build that writes a workspace ran out of space.
//! One boot reports both: the queue geometry each virtio device advertises, a write past the old
//! limit followed by a `sync` that must return, and the size `/tmp` was mounted at.

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
/// Bytes written to the writable root before the `sync` that used to never return.
///
/// The kernel merges writes into one request of up to `max_sectors_kb`, which is 1280 KiB on this
/// machine and was above the mebibyte the device used to cap a request at, so this is the write
/// that reproduced the stall. It is far below the fifteen gigabytes the tuning proof writes, so
/// the size is not what makes it a test.
const WRITE_BYTES: u64 = 256 * 1024 * 1024;
/// The `size_max` the device must advertise, in bytes: the largest request the parser answers.
const SIZE_MAX: u64 = 4 * 1024 * 1024;

/// The one command the proof runs, reported as `key=value` lines.
pub const SCRIPT: &str = r#"set -u
echo "TMP_FSTYPE=$(findmnt -no FSTYPE /tmp)"
echo "TMP_SIZE_KIB=$(df -k /tmp | awk 'NR==2 {print $2}')"
echo "RUN_FSTYPE=$(findmnt -no FSTYPE /run)"
echo "RUN_SIZE_KIB=$(df -k /run | awk 'NR==2 {print $2}')"
for d in /sys/block/vd*; do
  n=$(basename "$d")
  echo "QUEUE_${n}_MAX_SECTORS_KB=$(cat "$d/queue/max_sectors_kb")"
  echo "QUEUE_${n}_MAX_HW_SECTORS_KB=$(cat "$d/queue/max_hw_sectors_kb")"
  echo "QUEUE_${n}_MAX_SEGMENTS=$(cat "$d/queue/max_segments")"
  echo "QUEUE_${n}_MAX_SEGMENT_SIZE=$(cat "$d/queue/max_segment_size")"
done
mkdir -p /var/blkproof
dd if=/dev/zero of=/var/blkproof/fill bs=1M count=256 status=none
echo "BLK_WRITE_BYTES=$(wc -c < /var/blkproof/fill)"
sync
echo "BLK_SYNC=ok"
rm -rf /var/blkproof
echo "END""#;

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and the large OCI layout"]
fn the_large_shape_serves_a_write_past_a_mebibyte_and_sizes_its_scratch() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/bash",
        arguments: &[b"-c", SCRIPT.as_bytes()],
        // The write is a quarter of a gigabyte through the page cache onto the overlay. Before
        // the fix the `sync` behind it never returned, so this ceiling is what turns a stall into
        // a failure rather than a hung harness.
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
    eprintln!("[{LARGE}] block and scratch report:\n{stdout}");
    assert_block_transfer(&stdout);
    assert_scratch(&stdout);
}

/// Asserts the queue geometry every virtio block device advertises, and that a write past the
/// old limit completed and flushed.
fn assert_block_transfer(stdout: &str) {
    // `max_segment_size` is `size_max` read back from configuration space, so it is the one
    // number that proves the driver negotiated the feature and read the field. A device that
    // advertises nothing leaves the kernel's own UINT_MAX here, which is what let a driver form
    // a request the device then refused. `max_segments` is `seg_max`, and with one segment the
    // request is bounded by the segment size.
    let segments = queue_lines(stdout, "_MAX_SEGMENT_SIZE");
    assert!(
        !segments.is_empty(),
        "no virtio block device reported its queue: stdout={stdout:?}"
    );
    for (device, size) in &segments {
        assert_eq!(
            *size, SIZE_MAX,
            "{device} advertises max_segment_size={size}, not the four mebibytes it answers"
        );
    }
    for (device, count) in queue_lines(stdout, "_MAX_SEGMENTS") {
        assert_eq!(count, 1, "{device} advertises max_segments={count}");
    }
    // `max_hw_sectors_kb` is not ours: this kernel derives it from neither `size_max` nor
    // `seg_max`, so it stays at its own default and is reported rather than asserted. What the
    // device does bound is the request the driver may build, which is the segment size times the
    // segment count, and that is checked here against the parser's own limit.
    let sectors = queue_lines(stdout, "_MAX_SECTORS_KB");
    assert!(
        !sectors.is_empty(),
        "no virtio block device reported its request cap: stdout={stdout:?}"
    );
    for (device, kib) in &sectors {
        assert!(
            kib.saturating_mul(1024) <= SIZE_MAX,
            "{device} may merge {kib} KiB into one request, past the four mebibytes the device answers"
        );
        // The write below only crosses the old mebibyte cap if the driver is allowed more than
        // one mebibyte in a single request, so the cap is part of the proof rather than a detail.
        assert!(
            *kib > 1024,
            "{device} caps one request at {kib} KiB, so the write never crossed the old limit"
        );
    }

    assert_eq!(
        reported(stdout, "blk_write_bytes"),
        Some(WRITE_BYTES),
        "the guest did not write what it was told to: stdout={stdout:?}"
    );
    // The stall was a `sync` that never returned, so the boot finishing with this line printed
    // after it is the whole point.
    assert_eq!(
        line(stdout, "blk_sync"),
        Some("ok"),
        "the guest never came back from sync: stdout={stdout:?}"
    );
    assert!(stdout.contains("END"), "the guest script did not finish");
}

/// Asserts the scratch filesystem the large shape was given, and that `/run` did not move.
fn assert_scratch(stdout: &str) {
    assert_eq!(line(stdout, "tmp_fstype"), Some("tmpfs"));
    let tmp = reported(stdout, "tmp_size_kib").expect("no /tmp size reported");
    assert!(
        tmp >= 7 * 1024 * 1024,
        "/tmp is {tmp} KiB, which is not the large shape's half of RAM; 64 MiB is 65536 KiB"
    );
    // `/run` holds sockets and process state, not a workload, so the small machine's size stands.
    assert_eq!(line(stdout, "run_fstype"), Some("tmpfs"));
    assert_eq!(
        reported(stdout, "run_size_kib"),
        Some(16 * 1024),
        "/run must keep the size every machine has had: stdout={stdout:?}"
    );
}

/// Every `QUEUE_<device><suffix>=<number>` line, as its device name and value.
fn queue_lines<'a>(stdout: &'a str, suffix: &str) -> Vec<(&'a str, u64)> {
    stdout
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let device = name.strip_prefix("QUEUE_")?.strip_suffix(suffix)?;
            Some((device, value.trim().parse().ok()?))
        })
        .collect()
}

/// Reads one `key=value` line as its value, matching the key without regard to case.
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
    fn the_reader_finds_every_device_and_matches_a_key_whatever_its_case() {
        let report = "QUEUE_vda_MAX_HW_SECTORS_KB=4096\nQUEUE_vdb_MAX_HW_SECTORS_KB=4096\n\
                      TMP_SIZE_KIB=8203432\nBLK_SYNC=ok\nEND\n";
        let devices = queue_lines(report, "_MAX_HW_SECTORS_KB");
        assert_eq!(devices, vec![("vda", 4096), ("vdb", 4096)]);
        assert_eq!(reported(report, "tmp_size_kib"), Some(8_203_432));
        assert_eq!(line(report, "blk_sync"), Some("ok"));
        assert_eq!(line(report, "end"), None);
        assert!(queue_lines(report, "_MAX_SEGMENTS").is_empty());
    }
}
