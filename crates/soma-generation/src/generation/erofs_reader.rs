use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

mod dir;

pub(crate) use dir::Dirent;

const SUPERBLOCK_OFFSET: u64 = 1024;
const MAGIC: u32 = 0xE0F5_E1E2;
const LAYOUT_FLAT_PLAIN: u16 = 0;
const LAYOUT_FLAT_INLINE: u16 = 2;
pub(crate) const S_IFMT: u16 = 0o170_000;
pub(crate) const S_IFDIR: u16 = 0o040_000;
pub(crate) const S_IFREG: u16 = 0o100_000;
pub(crate) const S_IFLNK: u16 = 0o120_000;
pub(crate) const S_IFIFO: u16 = 0o010_000;

/// Superblock fields checked by the verifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SuperBlock {
    pub(crate) block_size: u64,
    pub(crate) root_nid: u64,
    pub(crate) inode_count: u64,
    pub(crate) build_time: u64,
    pub(crate) meta_block: u64,
    pub(crate) uuid: [u8; 16],
    pub(crate) volume_name: [u8; 16],
    pub(crate) feature_incompat: u32,
}

/// One decoded inode without any host-side interpretation of its name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Inode {
    pub(crate) nid: u64,
    pub(crate) mode: u16,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) nlink: u32,
    pub(crate) size: u64,
    pub(crate) mtime: u64,
    pub(crate) xattr_count: u16,
    layout: u16,
    raw_block: u64,
    inline_offset: u64,
}

/// A bounded userspace reader for uncompressed 4 KiB-block EROFS images.
///
/// It never mounts the image and rejects compressed, chunked, or xattr-bearing inodes.
pub(crate) struct ErofsImage {
    file: File,
    size: u64,
    pub(crate) superblock: SuperBlock,
}

impl ErofsImage {
    pub(crate) fn open(path: &Path, max_bytes: u64) -> Result<Self, CompileError> {
        Self::from_file(File::open(path).map_err(|_| io_error())?, max_bytes)
    }

    pub(crate) fn from_file(file: File, max_bytes: u64) -> Result<Self, CompileError> {
        let size = file.metadata().map_err(|_| io_error())?.len();
        if size > max_bytes {
            return Err(CompileError::new(
                CompilePhase::VerifyRoot,
                CompileErrorKind::LimitExceeded,
            ));
        }
        let mut image = Self {
            file,
            size,
            superblock: SuperBlock {
                block_size: 0,
                root_nid: 0,
                inode_count: 0,
                build_time: 0,
                meta_block: 0,
                uuid: [0; 16],
                volume_name: [0; 16],
                feature_incompat: 0,
            },
        };
        image.superblock = image.read_superblock()?;
        Ok(image)
    }

    fn read_superblock(&mut self) -> Result<SuperBlock, CompileError> {
        let raw = self.read_at(SUPERBLOCK_OFFSET, 128)?;
        if u32_at(&raw, 0)? != MAGIC {
            return Err(integrity());
        }
        if raw[12] != 12 {
            return Err(unsupported());
        }
        Ok(SuperBlock {
            block_size: 4096,
            root_nid: u64::from(u16::from_le_bytes([raw[14], raw[15]])),
            inode_count: u64_at(&raw, 16)?,
            build_time: u64_at(&raw, 24)?,
            meta_block: u64::from(u32_at(&raw, 40)?),
            uuid: raw[48..64].try_into().map_err(|_| integrity())?,
            volume_name: raw[64..80].try_into().map_err(|_| integrity())?,
            feature_incompat: u32_at(&raw, 80)?,
        })
    }

    pub(crate) fn inode(&mut self, nid: u64) -> Result<Inode, CompileError> {
        let block_size = self.superblock.block_size;
        let address = self
            .superblock
            .meta_block
            .checked_mul(block_size)
            .and_then(|base| base.checked_add(nid.checked_mul(32)?))
            .ok_or_else(integrity)?;
        let compact = self.read_at(address, 32)?;
        let format = u16::from_le_bytes([compact[0], compact[1]]);
        let xattr_count = u16::from_le_bytes([compact[2], compact[3]]);
        let layout = (format >> 1) & 7;
        if format & !0x0f != 0 {
            return Err(integrity());
        }
        if layout != LAYOUT_FLAT_PLAIN && layout != LAYOUT_FLAT_INLINE {
            return Err(unsupported());
        }
        let xattr_bytes = if xattr_count == 0 {
            0
        } else {
            12 + u64::from(xattr_count - 1) * 4
        };
        let (inode_len, mode, uid, gid, nlink, size, mtime) = if format & 1 == 0 {
            (
                32_u64,
                u16::from_le_bytes([compact[4], compact[5]]),
                u32::from(u16::from_le_bytes([compact[24], compact[25]])),
                u32::from(u16::from_le_bytes([compact[26], compact[27]])),
                u32::from(u16::from_le_bytes([compact[6], compact[7]])),
                u64::from(u32_at(&compact, 8)?),
                self.superblock.build_time,
            )
        } else {
            let extended = self.read_at(address, 64)?;
            (
                64_u64,
                u16::from_le_bytes([extended[4], extended[5]]),
                u32_at(&extended, 24)?,
                u32_at(&extended, 28)?,
                u32_at(&extended, 44)?,
                u64_at(&extended, 8)?,
                u64_at(&extended, 32)?,
            )
        };
        Ok(Inode {
            nid,
            mode,
            uid,
            gid,
            nlink,
            size,
            mtime,
            xattr_count,
            layout,
            raw_block: u64::from(u32_at(&compact, 16)?),
            inline_offset: address
                .checked_add(inode_len)
                .and_then(|end| end.checked_add(xattr_bytes))
                .ok_or_else(integrity)?,
        })
    }

    /// Reads logical bytes `[offset, offset + length)` of an inode's data.
    pub(crate) fn read_data(
        &mut self,
        inode: &Inode,
        offset: u64,
        length: usize,
    ) -> Result<Vec<u8>, CompileError> {
        let end = offset
            .checked_add(u64::try_from(length).map_err(|_| integrity())?)
            .ok_or_else(integrity)?;
        if end > inode.size {
            return Err(integrity());
        }
        let block_size = self.superblock.block_size;
        let inline_start = match inode.layout {
            LAYOUT_FLAT_INLINE if inode.size > 0 => inode.size.div_ceil(block_size) - 1,
            _ => u64::MAX / block_size,
        }
        .saturating_mul(block_size);
        let mut output = Vec::with_capacity(length);
        let mut cursor = offset;
        while cursor < end {
            let (physical, chunk_end) = if cursor >= inline_start {
                (inode.inline_offset + (cursor - inline_start), end)
            } else {
                (inode.raw_block * block_size + cursor, end.min(inline_start))
            };
            let count = usize::try_from(chunk_end - cursor).map_err(|_| integrity())?;
            output.extend_from_slice(&self.read_at(physical, count)?);
            cursor = chunk_end;
        }
        Ok(output)
    }

    pub(crate) fn hash_data(&mut self, inode: &Inode) -> Result<Sha256Digest, CompileError> {
        let mut hasher = Sha256::new();
        let mut offset = 0_u64;
        while offset < inode.size {
            let count =
                usize::try_from((inode.size - offset).min(1 << 20)).map_err(|_| integrity())?;
            hasher.update(self.read_data(inode, offset, count)?);
            offset += u64::try_from(count).map_err(|_| integrity())?;
        }
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(hasher.finalize().as_ref());
        Ok(Sha256Digest::from_bytes(digest))
    }

    pub(crate) fn read_dir(
        &mut self,
        inode: &Inode,
        max_entries: usize,
    ) -> Result<Vec<Dirent>, CompileError> {
        if inode.mode & S_IFMT != S_IFDIR {
            return Err(integrity());
        }
        let block_size = self.superblock.block_size;
        let mut entries = Vec::new();
        let mut offset = 0_u64;
        while offset < inode.size {
            let count =
                usize::try_from((inode.size - offset).min(block_size)).map_err(|_| integrity())?;
            let block = self.read_data(inode, offset, count)?;
            dir::parse_block(&block, &mut entries, max_entries)?;
            offset += u64::try_from(count).map_err(|_| integrity())?;
        }
        Ok(entries)
    }

    fn read_at(&mut self, offset: u64, length: usize) -> Result<Vec<u8>, CompileError> {
        let end = offset
            .checked_add(u64::try_from(length).map_err(|_| integrity())?)
            .ok_or_else(integrity)?;
        if end > self.size {
            return Err(integrity());
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|_| io_error())?;
        let mut buffer = vec![0_u8; length];
        self.file.read_exact(&mut buffer).map_err(|_| io_error())?;
        Ok(buffer)
    }
}

pub(super) fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, CompileError> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(integrity)?;
    Ok(u32::from_le_bytes(
        slice.try_into().map_err(|_| integrity())?,
    ))
}

pub(super) fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, CompileError> {
    let slice = bytes.get(offset..offset + 8).ok_or_else(integrity)?;
    Ok(u64::from_le_bytes(
        slice.try_into().map_err(|_| integrity())?,
    ))
}

pub(super) const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::VerifyRoot, CompileErrorKind::Integrity)
}

const fn unsupported() -> CompileError {
    CompileError::new(CompilePhase::VerifyRoot, CompileErrorKind::Unsupported)
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::VerifyRoot, CompileErrorKind::Io)
}
