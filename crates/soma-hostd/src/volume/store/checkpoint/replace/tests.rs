use super::*;
use crate::{RequestFingerprint, VolumeId, volume::VolumeStore};
use std::{
    fs::OpenOptions,
    io::{Seek as _, SeekFrom, Write as _},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
};

const TRANSITION: u64 = 11;

fn private(path: &Path, contents: &[u8]) -> File {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(contents).unwrap();
    file.sync_all().unwrap();
    file
}

fn stage(root: &Path) -> (PathBuf, PathBuf, Intent, File) {
    let backing = root.join(VOLUME_FILE);
    let path = root.join(".restore-test");
    fs::create_dir(&path).unwrap();
    let original = private(&backing, b"post-checkpoint writes");
    let replacement = private(&path.join(VOLUME_FILE), b"checkpoint bytes");
    let intent = Intent {
        fingerprint: [7; 32],
        original: inode(&original).unwrap(),
        replacement: inode(&replacement).unwrap(),
        transition: TRANSITION,
    };
    private(&path.join(INTENT), &intent.encode());
    (backing, path, intent, replacement)
}

#[test]
fn stage_requires_matching_intent_and_publication_requires_the_worker_lock() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, replacement) = stage(root.path());
    assert!(matches!(
        Intent::read(&path, [8; 32]),
        Err(VolumeError::ChangedIntent)
    ));
    let publish = |transition| {
        publish(
            root.path(),
            &path,
            &backing,
            intent,
            VolumeState::Detached,
            transition,
        )
    };
    assert!(matches!(publish(TRANSITION), Err(VolumeError::Busy(_))));
    lock(&replacement).unwrap();
    assert_eq!(publish(TRANSITION + 1), Err(VolumeError::ChangedIntent));
    assert_eq!(fs::read(backing).unwrap(), b"post-checkpoint writes");
    assert!(!path.join(RETAINED).exists());
}

#[test]
fn publication_preserves_newer_writes_and_can_resume_after_persisting_the_old_link() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, replacement) = stage(root.path());
    lock(&replacement).unwrap();
    let publish = || {
        publish(
            root.path(),
            &path,
            &backing,
            intent,
            VolumeState::Detached,
            TRANSITION,
        )
    };
    fs::hard_link(&backing, path.join(RETAINED)).unwrap();
    assert_eq!(fs::read(&backing).unwrap(), b"post-checkpoint writes");

    publish().unwrap();
    publish().unwrap();
    assert_eq!(fs::read(&backing).unwrap(), b"checkpoint bytes");
    assert_eq!(
        fs::read(path.join(RETAINED)).unwrap(),
        b"post-checkpoint writes"
    );
    assert!(!path.join(VOLUME_FILE).exists());
    assert_eq!(
        inode(&private_file(&backing, false).unwrap()).unwrap(),
        inode(&replacement).unwrap()
    );
}

#[test]
fn an_unrelated_retained_file_cannot_be_used_as_a_rollback_version() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, replacement) = stage(root.path());
    lock(&replacement).unwrap();
    private(&path.join(RETAINED), b"other");
    assert_eq!(
        publish(
            root.path(),
            &path,
            &backing,
            intent,
            VolumeState::Detached,
            TRANSITION
        ),
        Err(VolumeError::ChangedIntent)
    );
    assert_eq!(fs::read(backing).unwrap(), b"post-checkpoint writes");
    assert_eq!(
        fs::read(path.join(VOLUME_FILE)).unwrap(),
        b"checkpoint bytes"
    );
}

#[test]
fn rolling_back_requires_a_stopped_worker_and_keeps_its_later_writes() {
    let root = tempfile::tempdir().unwrap();
    let (backing, path, intent, mut replacement) = stage(root.path());
    lock(&replacement).unwrap();
    publish(
        root.path(),
        &path,
        &backing,
        intent,
        VolumeState::Detached,
        TRANSITION,
    )
    .unwrap();

    assert!(matches!(
        rollback(root.path(), &path, &backing, intent),
        Err(VolumeError::Busy(_))
    ));
    replacement.write_all(b" and recovered writes").unwrap();
    replacement.sync_all().unwrap();
    drop(replacement);

    fs::hard_link(&backing, path.join(DISPLACED)).unwrap();
    fs::hard_link(path.join(RETAINED), path.join(ROLLBACK)).unwrap();
    rollback(root.path(), &path, &backing, intent).unwrap();
    rollback(root.path(), &path, &backing, intent).unwrap();

    assert_eq!(fs::read(backing).unwrap(), b"post-checkpoint writes");
    assert_eq!(
        fs::read(path.join(DISPLACED)).unwrap(),
        b"checkpoint bytes and recovered writes"
    );
}

fn saved_workspace(saved_path: &Path) -> RequestFingerprint {
    let saved = private(saved_path, b"saved");
    saved.set_len(1 << 30).unwrap();
    drop(saved);
    let mut saved = private_file(saved_path, false).unwrap();
    let mut hash = Sha256::new();
    let mut chunk = vec![0; 1 << 20];
    for _ in 0..1024 {
        saved.read_exact(&mut chunk).unwrap();
        hash.update(&chunk);
    }
    RequestFingerprint::new(hash.finalize().into()).unwrap()
}

fn competing_workspace_mutations_wait(store: &DirectoryVolumes, request: &WorkspaceRestoreRequest) {
    let busy = VolumeError::Busy("a workspace restore is staged");
    assert_eq!(store.destroy(request.volume), Err(busy.clone()));
    assert!(matches!(
        store.binding(request.checkpoint.source),
        Err(error) if error == busy
    ));
    let mut wrong_volume = *request;
    wrong_volume.volume = VolumeId::new([11; 16]).unwrap();
    assert!(matches!(
        store.stage_workspace_restore(&wrong_volume),
        Err(VolumeError::ChangedIntent)
    ));
    let mut other = *request;
    other.operation = OperationId::new([19; 16]).unwrap();
    assert!(matches!(
        store.stage_workspace_from(&other, || panic!("a competing restore must not read the archive")),
        Err(error) if error == busy
    ));
}

#[test]
fn a_restarted_store_can_publish_then_roll_back_the_exact_operation() {
    use crate::{
        GenerationId, InstanceId,
        volume::{CheckpointVolumeRequest, CleanRepair},
    };

    let root = tempfile::tempdir().unwrap();
    let volumes = root.path().join("volumes");
    let records = root.path().join("records");
    let open = || {
        DirectoryVolumes::open_uncertified(&volumes, &records, Box::new(CleanRepair), true).unwrap()
    };
    let instance = InstanceId::new([4; 16]).unwrap();
    let operation = OperationId::new([5; 16]).unwrap();
    let volume = VolumeId::new([6; 16]).unwrap();
    let generation = GenerationId::new([7; 32]).unwrap();
    let saved_path = root.path().join("saved.img");
    let digest = saved_workspace(&saved_path);
    let request = WorkspaceRestoreRequest {
        operation,
        volume,
        checkpoint: CheckpointVolumeRequest {
            source: instance,
            destination: instance,
            checkpoint: OperationId::new([8; 16]).unwrap(),
            generation,
            digest,
        },
    };
    let store = open();
    let directory = store.directory(volume);
    let backing = store.file(volume);
    let path = restore_path(&directory, operation);
    fs::create_dir(&directory).unwrap();
    let sidecar = crate::VolumeSidecar {
        volume,
        instance,
        generation,
        tenant: String::from("fixture"),
        size_class_gib: 1,
        state: VolumeState::Detached,
        dek_id: [1; 16],
        created_at_nanos: 1,
        last_transition_nanos: TRANSITION,
    };
    private(
        &directory.join(crate::volume::SIDECAR_FILE),
        &sidecar.encode().unwrap(),
    );
    let original = private(&backing, b"newer");
    original
        .set_permissions(fs::Permissions::from_mode(0o644))
        .unwrap();
    original.set_len(1 << 30).unwrap();
    drop(store);
    let store = open();
    let fresh = || private_file(&saved_path, false);
    let first = store.stage_workspace_from(&request, fresh).unwrap();
    competing_workspace_mutations_wait(&store, &request);
    let active = private_file(&first.path, true).unwrap();
    lock(&active).unwrap();
    assert!(matches!(
        store.abort_workspace_restore(&request),
        Err(VolumeError::Busy(_))
    ));
    drop(active);
    store.abort_workspace_restore(&request).unwrap();
    store.abort_workspace_restore(&request).unwrap();
    assert!(!path.exists());

    let prepared = store.stage_workspace_from(&request, fresh).unwrap();
    drop(store);

    let store = open();
    let binding = store.stage_workspace_restore(&request).unwrap();
    assert_eq!(binding.volume, volume);
    assert_eq!(binding.path, prepared.path);
    let replacement = private_file(&binding.path, true).unwrap();
    let mut changed = request;
    changed.checkpoint.digest = RequestFingerprint::new([10; 32]).unwrap();
    assert!(matches!(
        store.stage_workspace_restore(&changed),
        Err(VolumeError::ChangedIntent)
    ));

    lock(&replacement).unwrap();
    store.publish_workspace_restore(&request).unwrap();
    store.abort_workspace_restore(&request).unwrap();
    assert!(path.join(RETAINED).is_file());
    drop(store);
    drop(replacement);

    let store = open();
    store.publish_workspace_restore(&request).unwrap();
    store.rollback_workspace_restore(&request).unwrap();
    let mut marker = [0; 5];
    open_regular(&backing, false)
        .unwrap()
        .read_exact(&mut marker)
        .unwrap();
    assert_eq!(&marker, b"newer");
}

#[test]
fn exact_pending_restore_recovers_both_durable_publication_cuts() {
    use crate::{
        GenerationId, InstanceId,
        volume::{CheckpointVolumeRequest, CleanRepair},
    };

    for attached in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let volumes = root.path().join("volumes");
        let records = root.path().join("records");
        let open = || {
            DirectoryVolumes::open_uncertified(&volumes, &records, Box::new(CleanRepair), true)
                .unwrap()
        };
        let instance = InstanceId::new([1; 16]).unwrap();
        let volume = VolumeId::new([2; 16]).unwrap();
        let generation = GenerationId::new([3; 32]).unwrap();
        let request = WorkspaceRestoreRequest {
            operation: OperationId::new([4; 16]).unwrap(),
            volume,
            checkpoint: CheckpointVolumeRequest {
                source: instance,
                destination: instance,
                generation,
                checkpoint: OperationId::new([5; 16]).unwrap(),
                digest: RequestFingerprint::new([6; 32]).unwrap(),
            },
        };
        let store = open();
        let directory = store.directory(volume);
        fs::create_dir(&directory).unwrap();
        let sidecar = crate::VolumeSidecar {
            volume,
            instance,
            generation,
            tenant: String::from("fixture"),
            size_class_gib: 1,
            state: VolumeState::Detached,
            dek_id: [1; 16],
            created_at_nanos: 1,
            last_transition_nanos: TRANSITION,
        };
        store.write_sidecar(&sidecar).unwrap();
        let (backing, placeholder, mut intent, mut worker) = stage(&directory);
        let path = restore_path(&directory, request.operation);
        fs::set_permissions(&placeholder, fs::Permissions::from_mode(0o700)).unwrap();
        fs::rename(&placeholder, &path).unwrap();
        intent.fingerprint = store.restore_fingerprint(&request).unwrap();
        fs::write(path.join(INTENT), intent.encode()).unwrap();
        drop(store);

        let store = open();
        lock(&worker).unwrap();
        if attached {
            store.activate_workspace_restore(&request).unwrap();
        } else {
            interruption::arm(&store, &path).unwrap();
            store.publish_workspace_restore(&request).unwrap();
        }
        assert!(matches!(
            store.reconcile_workspace_restore(&request),
            Err(VolumeError::Busy(_))
        ));
        worker.seek(SeekFrom::End(0)).unwrap();
        worker.write_all(b" with later writes").unwrap();
        worker.sync_all().unwrap();
        drop(worker);
        refuse_changed_publication_revision(store, &open, &request);

        let store = open();
        let mut changed = request;
        changed.checkpoint.digest = RequestFingerprint::new([8; 32]).unwrap();
        assert_eq!(
            store.reconcile_workspace_restore(&changed),
            Err(VolumeError::ChangedIntent)
        );
        store.reconcile_workspace_restore(&request).unwrap();
        assert_eq!(store.inspect(volume).unwrap(), sidecar);
        assert_eq!(fs::read(&backing).unwrap(), b"post-checkpoint writes");
        assert!(!path.exists());
        let history = fs::read_dir(&directory)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".restore-history-")
            })
            .unwrap();
        assert_eq!(
            fs::read(history.join(DISPLACED)).unwrap(),
            b"checkpoint bytes with later writes"
        );
        store.reconcile_workspace_restore(&request).unwrap();
    }
}

fn refuse_changed_publication_revision(
    store: DirectoryVolumes,
    open: &impl Fn() -> DirectoryVolumes,
    request: &WorkspaceRestoreRequest,
) {
    let persisted = store.inspect(request.volume).unwrap();
    let changed = crate::VolumeSidecar {
        last_transition_nanos: persisted.last_transition_nanos + 1,
        ..persisted.clone()
    };
    store.write_sidecar(&changed).unwrap();
    drop(store);
    let store = open();
    assert_eq!(
        store.reconcile_workspace_restore(request),
        Err(VolumeError::ChangedIntent)
    );
    store.write_sidecar(&persisted).unwrap();
}
