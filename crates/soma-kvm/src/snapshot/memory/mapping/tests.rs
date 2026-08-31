//! Proofs of the mapping's contract: private writes, refused inputs, a prefault that
//! changes nothing, and a handover that unmaps exactly once.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use super::{MappingError, PrivateMapping, page_size};

struct TempFile(PathBuf);

impl TempFile {
    fn create(content: &[u8]) -> (Self, File) {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "soma-snapshot-mapping-{}-{nanos}.raw",
            process::id()
        ));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(content).unwrap();
        file.sync_all().unwrap();
        (Self(path), file)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[test]
fn writes_through_one_private_mapping_are_invisible_to_a_sibling_and_the_file() {
    let original = vec![0x5a_u8; 8192];
    let (_guard, mut file) = TempFile::create(&original);
    let mut first = PrivateMapping::map(&file, 8192).unwrap();
    let second = PrivateMapping::map(&file, 8192).unwrap();
    assert_eq!(first.len(), 8192);
    assert!(!first.is_empty());
    assert_ne!(first.as_ptr(), second.as_ptr());
    assert_eq!(first.as_slice(), &original[..]);

    first.as_mut_slice()[0] = 0xa5;
    first.as_mut_slice()[4096] = 0xa5;
    assert_eq!(first.as_slice()[0], 0xa5);
    assert_eq!(second.as_slice(), &original[..]);

    let mut on_disk = Vec::new();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_to_end(&mut on_disk).unwrap();
    assert_eq!(on_disk, original);
    drop(first);
    assert_eq!(second.as_slice(), &original[..]);
}

#[test]
fn rejects_zero_length_and_short_files() {
    let (_guard, file) = TempFile::create(&[1; 4096]);
    assert_eq!(
        PrivateMapping::map(&file, 0).unwrap_err(),
        MappingError::ZeroLength
    );
    assert_eq!(
        PrivateMapping::map(&file, 4097).unwrap_err(),
        MappingError::FileShorterThanMapping {
            file_len: 4096,
            requested: 4097
        }
    );
    assert_eq!(
        PrivateMapping::map(&file, u64::MAX).unwrap_err(),
        MappingError::LengthExceedsAddressSpace(u64::MAX)
    );
    let directory = File::open(std::env::temp_dir()).unwrap();
    assert_eq!(
        PrivateMapping::map(&directory, 4096).unwrap_err(),
        MappingError::NotRegularFile
    );
}

#[test]
fn a_prefault_touches_every_page_and_changes_nothing() {
    let original = vec![0x11_u8; 12288];
    let (_guard, file) = TempFile::create(&original);
    let mapping = PrivateMapping::map(&file, 12288).unwrap();
    assert_eq!(mapping.prefault(), 12288 / page_size());
    assert_eq!(mapping.as_slice(), &original[..]);
    assert_eq!(mapping.prefault(), mapping.prefault());
}

#[test]
fn into_raw_hands_over_the_exact_range_and_suppresses_the_unmap() {
    let (_guard, file) = TempFile::create(&[3; 8192]);
    let mapping = PrivateMapping::map(&file, 8192).unwrap();
    let expected = mapping.as_ptr();
    let (base, len) = mapping.into_raw();
    assert_eq!(base, expected);
    assert_eq!(len, 8192);
    // SAFETY: `base` and `len` are exactly the range `into_raw` released, and this is the
    // only unmap of that range because `into_raw` suppressed the mapping's own `Drop`.
    assert_eq!(unsafe { libc::munmap(base.cast(), len) }, 0);
}
