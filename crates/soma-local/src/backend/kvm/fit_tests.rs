use soma::{MachineShape, VolumeAttachment};

use super::{Booted, paired_volume_misfit};

const GENERATION: Booted = Booted {
    vcpus: 2,
    boot_mib: 4096,
    storage_bytes: 20480 * 1024 * 1024,
};

fn shape(vcpus: u16, memory_mib: u64, storage_mib: u64) -> MachineShape {
    MachineShape::new(vcpus, memory_mib, storage_mib).expect("shape")
}

#[test]
fn a_request_the_generation_gives_has_no_reason() {
    assert_eq!(GENERATION.misfit(&shape(2, 4096, 20480), |_| false), None);
    assert_eq!(GENERATION.misfit(&shape(1, 8192, 20480), |_| true), None);
}

#[test]
fn each_refusal_names_the_field_the_request_and_the_generation() {
    let cases = [
        (shape(4, 4096, 20480), "vcpu_count 4 is outside 1..=2"),
        (
            shape(2, 2048, 20480),
            "memory_mib 2048 is below the Generation's boot memory 4096",
        ),
        (
            shape(2, 8192, 20480),
            "memory_mib 8192 is above the Generation's boot memory 4096",
        ),
        (
            shape(2, 4096, 10240),
            "storage_mib 10240 is not the Generation's writable storage",
        ),
    ];
    for (request, expected) in cases {
        let reason = GENERATION.misfit(&request, |_| false).expect("refused");
        assert!(reason.contains(expected), "{reason}");
    }
}

#[test]
fn a_volume_the_generation_cannot_take_is_named() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("volume.img");
    std::fs::File::create(&path)
        .and_then(|file| file.set_len(10 << 30))
        .expect("sparse volume");
    let volume = VolumeAttachment::new([7; 16], &path);

    assert_eq!(paired_volume_misfit(false, 10, None), None);
    // SOMA#262: a volume-declaring Generation launched without a volume gets a scratch disk.
    assert_eq!(paired_volume_misfit(true, 10, None), None);
    assert_eq!(paired_volume_misfit(true, 10, Some(&volume)), None);
    let cases = [
        (
            paired_volume_misfit(false, 10, Some(&volume)),
            "declares no persistent volume",
        ),
        (
            paired_volume_misfit(true, 20, Some(&volume)),
            "the Generation's volume is 20 GiB",
        ),
    ];
    for (reason, expected) in cases {
        let reason = reason.expect("refused");
        assert!(reason.contains(expected), "{reason}");
    }
}
