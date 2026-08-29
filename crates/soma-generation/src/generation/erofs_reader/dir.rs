use super::{integrity, u64_at};
use crate::generation::error::{CompileError, CompileErrorKind, CompilePhase};

const DIRENT_LEN: usize = 12;
const MAX_NAME: usize = 255;

/// One directory entry as stored on disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Dirent {
    pub(crate) nid: u64,
    pub(crate) file_type: u8,
    pub(crate) name: Vec<u8>,
}

/// Parses one EROFS directory block: a dirent array followed by packed names.
///
/// The first entry's name offset fixes the entry count; every name must lie after the array,
/// inside the block, be non-empty, be at most 255 bytes, and contain no `/`.
pub(super) fn parse_block(
    block: &[u8],
    entries: &mut Vec<Dirent>,
    max_entries: usize,
) -> Result<(), CompileError> {
    if block.len() < DIRENT_LEN {
        return Err(integrity());
    }
    let first_name = usize::from(u16::from_le_bytes([block[8], block[9]]));
    if !first_name.is_multiple_of(DIRENT_LEN) || first_name == 0 || first_name > block.len() {
        return Err(integrity());
    }
    let count = first_name / DIRENT_LEN;
    for index in 0..count {
        let dirent = &block[index * DIRENT_LEN..(index + 1) * DIRENT_LEN];
        let nid = u64_at(dirent, 0)?;
        let name_start = usize::from(u16::from_le_bytes([dirent[8], dirent[9]]));
        let name_end = if index + 1 < count {
            let next = &block[(index + 1) * DIRENT_LEN..(index + 2) * DIRENT_LEN];
            usize::from(u16::from_le_bytes([next[8], next[9]]))
        } else {
            block[name_start.min(block.len())..]
                .iter()
                .position(|byte| *byte == 0)
                .map_or(block.len(), |position| name_start + position)
        };
        if name_start < first_name || name_end > block.len() || name_end <= name_start {
            return Err(integrity());
        }
        let name = block[name_start..name_end].to_vec();
        if name.len() > MAX_NAME || name.contains(&b'/') {
            return Err(integrity());
        }
        if entries.len() >= max_entries {
            return Err(CompileError::new(
                CompilePhase::VerifyRoot,
                CompileErrorKind::LimitExceeded,
            ));
        }
        entries.push(Dirent {
            nid,
            file_type: dirent[10],
            name,
        });
    }
    Ok(())
}
