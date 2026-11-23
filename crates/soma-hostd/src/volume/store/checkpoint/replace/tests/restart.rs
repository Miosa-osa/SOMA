//! A restarted store resumes, publishes and rolls back the exact pending restore.

use super::*;

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
