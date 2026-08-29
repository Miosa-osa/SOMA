use std::collections::BTreeMap;

use super::{
    entry::Metadata,
    tree::Tree,
    tree_model::{FileNode, Node},
};
use crate::{NormalizeError, NormalizeErrorKind, NormalizePhase};

pub(crate) const MEDIA_TYPE: &str = "application/vnd.soma.rootfs-tree.v1";
const MAGIC: &[u8; 8] = b"SOMARFS\0";

pub(super) fn encode(tree: &Tree, maximum: u64) -> Result<Vec<u8>, NormalizeError> {
    let mut anchors = BTreeMap::<u64, &[u8]>::new();
    for (path, node) in &tree.entries {
        if let Node::Regular(file) = node {
            anchors.entry(file.inode).or_insert(path);
        }
    }
    let mut encoder = Encoder::new(maximum)?;
    encoder.bytes(MAGIC)?;
    encoder.u16(1)?;
    encoder.u16(1)?;
    encoder.u32(u32::try_from(tree.entries.len()).map_err(|_| limit())?)?;
    for (path, node) in &tree.entries {
        encoder.sized_bytes(path)?;
        match node {
            Node::Directory(metadata) => {
                encoder.u8(1)?;
                encoder.metadata(metadata)?;
            }
            Node::Regular(file) => {
                let anchor = anchors.get(&file.inode).ok_or_else(integrity)?;
                if path.as_slice() == *anchor {
                    encode_regular(&mut encoder, file)?;
                } else {
                    encoder.u8(5)?;
                    encoder.metadata(&file.metadata)?;
                    encoder.sized_bytes(anchor)?;
                }
            }
            Node::Symlink { metadata, target } => {
                encoder.u8(3)?;
                encoder.metadata(metadata)?;
                encoder.sized_bytes(target)?;
            }
            Node::Fifo(metadata) => {
                encoder.u8(4)?;
                encoder.metadata(metadata)?;
            }
        }
    }
    Ok(encoder.finish())
}

fn encode_regular(encoder: &mut Encoder, file: &FileNode) -> Result<(), NormalizeError> {
    encoder.u8(2)?;
    encoder.metadata(&file.metadata)?;
    encoder.u64(file.size)?;
    encoder.bytes(&digest_bytes(file.digest.as_str())?)
}

fn digest_bytes(value: &str) -> Result<[u8; 32], NormalizeError> {
    let hex = value.strip_prefix("sha256:").ok_or_else(integrity)?;
    if hex.len() != 64 {
        return Err(integrity());
    }
    let mut output = [0_u8; 32];
    for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        output[index] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(output)
}

fn nibble(value: u8) -> Result<u8, NormalizeError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(integrity()),
    }
}

struct Encoder {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Encoder {
    fn new(maximum: u64) -> Result<Self, NormalizeError> {
        Ok(Self {
            bytes: Vec::new(),
            maximum: usize::try_from(maximum).map_err(|_| limit())?,
        })
    }

    fn metadata(&mut self, value: &Metadata) -> Result<(), NormalizeError> {
        self.u32(value.mode)?;
        self.u32(value.uid)?;
        self.u32(value.gid)?;
        self.u64(value.mtime)?;
        self.u32(0)
    }

    fn sized_bytes(&mut self, value: &[u8]) -> Result<(), NormalizeError> {
        self.u32(u32::try_from(value.len()).map_err(|_| limit())?)?;
        self.bytes(value)
    }

    fn u8(&mut self, value: u8) -> Result<(), NormalizeError> {
        self.bytes(&[value])
    }

    fn u16(&mut self, value: u16) -> Result<(), NormalizeError> {
        self.bytes(&value.to_be_bytes())
    }

    fn u32(&mut self, value: u32) -> Result<(), NormalizeError> {
        self.bytes(&value.to_be_bytes())
    }

    fn u64(&mut self, value: u64) -> Result<(), NormalizeError> {
        self.bytes(&value.to_be_bytes())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), NormalizeError> {
        let length = self
            .bytes
            .len()
            .checked_add(value.len())
            .ok_or_else(limit)?;
        if length > self.maximum {
            return Err(limit());
        }
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

const fn integrity() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::EncodeManifest,
        NormalizeErrorKind::Integrity,
    )
}

const fn limit() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::EncodeManifest,
        NormalizeErrorKind::LimitExceeded,
    )
}
