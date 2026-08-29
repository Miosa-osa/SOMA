use std::io::{Read, Write};

use sha2::{Digest as _, Sha256};

use super::{
    artifacts::Sha256Digest,
    error::{CompileError, CompileErrorKind, CompilePhase},
    tree_decoder::{TreeBounds, TreeDecoder, TreeEntry, TreeNode, TreeSummary},
};
use crate::{ImportPhase, normalize::CONTENT_MEDIA_TYPE, oci::Descriptor, store::Store};

const BLOCK: usize = 512;
const MAX_OCTAL_7: u64 = 0o777_7777;
const MAX_OCTAL_11: u64 = 0o7_7777_7777_7777;

/// Emits the canonical tree as an ordered `ustar` stream with local PAX overrides.
///
/// Guest paths remain bytes inside the archive and never become host paths.
/// Regular-file bodies stream from verified content objects and are re-hashed while copying.
pub(crate) fn stream_tree(
    manifest: &[u8],
    bounds: TreeBounds,
    max_output_bytes: u64,
    store: &Store,
    writer: &mut dyn Write,
) -> Result<TreeSummary, CompileError> {
    let mut decoder = TreeDecoder::new(manifest, bounds)?;
    let mut sink = Sink {
        writer,
        written: 0,
        maximum: max_output_bytes,
    };
    for entry in decoder.by_ref() {
        let entry = entry?;
        emit_entry(&entry, store, &mut sink)?;
    }
    sink.write(&[0_u8; BLOCK * 2])?;
    decoder.finish()
}

fn emit_entry(entry: &TreeEntry, store: &Store, sink: &mut Sink<'_>) -> Result<(), CompileError> {
    let name: &[u8] = if entry.path.is_empty() {
        b"./"
    } else {
        &entry.path
    };
    let (kind, link, size): (u8, &[u8], u64) = match &entry.node {
        TreeNode::Directory => (b'5', b"", 0),
        TreeNode::Regular { size, .. } => (b'0', b"", *size),
        TreeNode::Symlink { target } => (b'2', target, 0),
        TreeNode::Hardlink { anchor } => (b'1', anchor, 0),
        TreeNode::Fifo => (b'6', b"", 0),
    };
    let mut pax = Vec::new();
    if name.len() > 100 {
        pax_record(&mut pax, b"path", name);
    }
    if link.len() > 100 {
        pax_record(&mut pax, b"linkpath", link);
    }
    if u64::from(entry.uid) > MAX_OCTAL_7 {
        pax_record(&mut pax, b"uid", entry.uid.to_string().as_bytes());
    }
    if u64::from(entry.gid) > MAX_OCTAL_7 {
        pax_record(&mut pax, b"gid", entry.gid.to_string().as_bytes());
    }
    if size > MAX_OCTAL_11 {
        pax_record(&mut pax, b"size", size.to_string().as_bytes());
    }
    if entry.mtime > MAX_OCTAL_11 {
        pax_record(&mut pax, b"mtime", entry.mtime.to_string().as_bytes());
    }
    if !pax.is_empty() {
        let length = u64::try_from(pax.len()).map_err(|_| limit())?;
        sink.write(&header(b"PaxHeader", 0o644, 0, 0, length, 0, b'x', b""))?;
        sink.write(&pax)?;
        sink.pad()?;
    }
    sink.write(&header(
        &name[..name.len().min(100)],
        entry.mode,
        u64::from(entry.uid).min(MAX_OCTAL_7),
        u64::from(entry.gid).min(MAX_OCTAL_7),
        size.min(MAX_OCTAL_11),
        entry.mtime.min(MAX_OCTAL_11),
        kind,
        &link[..link.len().min(100)],
    ))?;
    if let TreeNode::Regular { size, digest } = &entry.node {
        copy_body(store, *size, *digest, sink)?;
        sink.pad()?;
    }
    Ok(())
}

fn copy_body(
    store: &Store,
    size: u64,
    digest: Sha256Digest,
    sink: &mut Sink<'_>,
) -> Result<(), CompileError> {
    let descriptor = Descriptor {
        media_type: CONTENT_MEDIA_TYPE.to_owned(),
        digest: digest.to_oci(),
        size,
        platform: None,
    };
    let mut file = store
        .open_blob(&descriptor, ImportPhase::Publish)
        .map_err(|error| CompileError::from_import(CompilePhase::StreamTree, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut remaining = size;
    while remaining > 0 {
        let capacity = usize::try_from(remaining.min(64 * 1024)).map_err(|_| limit())?;
        let count = file.read(&mut buffer[..capacity]).map_err(|_| io_error())?;
        if count == 0 {
            return Err(integrity());
        }
        hasher.update(&buffer[..count]);
        sink.write(&buffer[..count])?;
        remaining -= u64::try_from(count).map_err(|_| limit())?;
    }
    if file.read(&mut [0_u8]).map_err(|_| io_error())? != 0 {
        return Err(integrity());
    }
    let mut actual = [0_u8; 32];
    actual.copy_from_slice(hasher.finalize().as_ref());
    if actual != *digest.as_bytes() {
        return Err(integrity());
    }
    Ok(())
}

fn pax_record(output: &mut Vec<u8>, key: &[u8], value: &[u8]) {
    // The record length counts its own decimal digits, the space, `key=value`, and the newline.
    let body = key.len() + value.len() + 3;
    let mut length = body + 1;
    while length != body + digits(length) {
        length = body + digits(length);
    }
    output.extend_from_slice(length.to_string().as_bytes());
    output.push(b' ');
    output.extend_from_slice(key);
    output.push(b'=');
    output.extend_from_slice(value);
    output.push(b'\n');
}

fn digits(value: usize) -> usize {
    value.to_string().len()
}

#[allow(clippy::too_many_arguments)]
fn header(
    name: &[u8],
    mode: u32,
    uid: u64,
    gid: u64,
    size: u64,
    mtime: u64,
    kind: u8,
    link: &[u8],
) -> [u8; BLOCK] {
    let mut block = [0_u8; BLOCK];
    block[..name.len()].copy_from_slice(name);
    octal(&mut block[100..108], u64::from(mode & 0o7777));
    octal(&mut block[108..116], uid);
    octal(&mut block[116..124], gid);
    octal(&mut block[124..136], size);
    octal(&mut block[136..148], mtime);
    block[148..156].copy_from_slice(b"        ");
    block[156] = kind;
    block[157..157 + link.len()].copy_from_slice(link);
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    octal(&mut block[329..337], 0);
    octal(&mut block[337..345], 0);
    let checksum: u64 = block.iter().map(|byte| u64::from(*byte)).sum();
    octal(&mut block[148..155], checksum);
    block[155] = b' ';
    block
}

fn octal(field: &mut [u8], value: u64) {
    let width = field.len() - 1;
    let text = format!("{value:0width$o}");
    field[..width].copy_from_slice(&text.as_bytes()[text.len() - width..]);
    field[width] = 0;
}

struct Sink<'a> {
    writer: &'a mut dyn Write,
    written: u64,
    maximum: u64,
}

impl Sink<'_> {
    fn write(&mut self, bytes: &[u8]) -> Result<(), CompileError> {
        let length = u64::try_from(bytes.len()).map_err(|_| limit())?;
        self.written = self.written.checked_add(length).ok_or_else(limit)?;
        if self.written > self.maximum {
            return Err(limit());
        }
        self.writer.write_all(bytes).map_err(|_| io_error())
    }

    fn pad(&mut self) -> Result<(), CompileError> {
        let remainder = usize::try_from(self.written % BLOCK as u64).map_err(|_| limit())?;
        if remainder != 0 {
            self.write(&[0_u8; BLOCK][..BLOCK - remainder])?;
        }
        Ok(())
    }
}

const fn limit() -> CompileError {
    CompileError::new(CompilePhase::StreamTree, CompileErrorKind::LimitExceeded)
}

const fn integrity() -> CompileError {
    CompileError::new(CompilePhase::StreamTree, CompileErrorKind::Integrity)
}

const fn io_error() -> CompileError {
    CompileError::new(CompilePhase::StreamTree, CompileErrorKind::Io)
}
