use std::{fs::OpenOptions, path::PathBuf, process::Command};

use soma::VolumeAttachment;

use super::{
    Anchors, InstanceId, Jailed, Launching, check_filters, execute, fresh16, generation_bytes,
    instance_bytes, prepared,
};

#[test]
#[ignore = "requires isolated KVM, e2fsprogs, and a certified persistent Node 22 Generation"]
fn sealed_worker_reopens_its_workspace_after_graceful_stop() {
    let anchors = Anchors::configured().expect("isolated test jail");
    let prepared_root =
        PathBuf::from(std::env::var_os("SOMA_GENERATION_STORE").expect("prepared store"));
    let prepared = prepared::find(Some(&prepared_root), "node:22").expect("verified Generation");
    assert!(prepared.manifest.persistence.enabled);
    let root = std::env::var_os("SOMA_WORKSPACE_TEST_DIR").expect("private durable test directory");
    let scratch = tempfile::tempdir_in(root).expect("fixture directory");
    let volume_path = scratch.path().join("volume.img");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&volume_path)
        .unwrap();
    file.set_len(u64::from(prepared.manifest.persistence.size_class_gib) * 1024 * 1024 * 1024)
        .unwrap();
    drop(file);
    assert!(
        Command::new("/usr/sbin/mkfs.ext4")
            .arg("-q")
            .arg(&volume_path)
            .status()
            .unwrap()
            .success()
    );
    let volume = VolumeAttachment::new(fresh16(), volume_path);
    let instance = InstanceId::new(format!("{:032x}", u128::from_be_bytes(fresh16()))).unwrap();
    let launch = || {
        Jailed::launch(
            &anchors,
            &Launching {
                prepared: &prepared,
                instance: &instance,
                instance_bytes: instance_bytes(&instance).unwrap(),
                generation_bytes: generation_bytes(&prepared.id).unwrap(),
                memory_mib: prepared.manifest.shape.memory_bytes / (1024 * 1024),
                disk_mib: prepared.manifest.template.writable_storage_bytes / (1024 * 1024),
                checkpoint: None,
                volume: Some(&volume),
            },
        )
        .expect("launch sealed worker with certified volume")
    };
    let mut first = launch();
    check_filters(&first);
    let first_pid = first.handle.as_ref().unwrap().pid();
    execute(
        &mut first,
        b"/usr/local/bin/node",
        &[
            b"-e",
            br"
const fs = require('node:fs');
const fd = fs.openSync('/workspace/soma-persistence-proof', 'wx');
fs.writeSync(fd, 'workspace persisted');
fs.fsyncSync(fd);
fs.closeSync(fd);
fs.writeFileSync('/root/soma-disposable-proof', 'ephemeral');
",
        ],
    );
    assert_eq!(first.release(false).unwrap(), soma::CleanupMethod::Graceful);
    assert!(!PathBuf::from(format!("/proc/{first_pid}")).exists());

    // The managed path performs the same offline e2fsck before Start.
    let repaired = Command::new("/usr/sbin/e2fsck")
        .args(["-f", "-p"])
        .arg(&volume.path)
        .status()
        .unwrap();
    assert!(matches!(repaired.code(), Some(0..=2)));
    let mut second = launch();
    check_filters(&second);
    let read = execute(
        &mut second,
        b"/usr/local/bin/node",
        &[
            b"-e",
            br"
const fs = require('node:fs');
if (fs.existsSync('/root/soma-disposable-proof')) process.exit(2);
process.stdout.write(fs.readFileSync('/workspace/soma-persistence-proof'));
",
        ],
    );
    assert_eq!(read, b"workspace persisted");
    assert_eq!(
        second.release(false).unwrap(),
        soma::CleanupMethod::Graceful
    );
    eprintln!(
        "[workspace] Generation={} /workspace payload survived new sealed worker; root overlay replaced; both workers stopped cleanly",
        prepared.id.as_str()
    );
}
