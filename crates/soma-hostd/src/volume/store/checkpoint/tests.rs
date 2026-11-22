use super::*;

#[test]
fn a_checkpoint_copy_keeps_sparse_bytes_independent_and_rejects_a_bad_digest() {
    let directory = tempfile::tempdir().unwrap();
    let mut source = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.path().join("source"))
        .unwrap();
    let mut bytes = vec![0; 3 * (1 << 20)];
    bytes[17..22].copy_from_slice(b"saved");
    source.write_all(&bytes).unwrap();
    source.rewind().unwrap();
    let digest = Sha256::digest(&bytes).into();
    let stage = Staging::new(directory.path()).unwrap();
    let target = stage.0.join(VOLUME_FILE);
    clone_sparse(
        source.try_clone().unwrap(),
        &target,
        bytes.len() as u64,
        &digest,
    )
    .unwrap();
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert!(fs::metadata(&target).unwrap().blocks() * 512 < bytes.len() as u64);
    fs::write(&target, b"changed").unwrap();
    source.rewind().unwrap();
    let mut actual = Vec::new();
    source.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, bytes);
    source.rewind().unwrap();
    let bad = stage.0.join("bad");
    assert_eq!(
        clone_sparse(source, &bad, bytes.len() as u64, &[0; 32]),
        Err(BAD_SNAPSHOT)
    );
    drop(stage);
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
}
