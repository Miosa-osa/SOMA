//! A compiled Generation is a pure function of its inputs, so it is built once and reopened.
//!
//! Compiling `node:22` writes an EROFS root above a gigabyte and takes minutes, and every live
//! run of every test binary repeated that work for bytes that never differ. The cache keys the
//! finished store on every input that can change the result and reopens it instead.

use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    path::Path,
    time::{Duration, SystemTime},
};

use sha2::{Digest as _, Sha256};
use soma_generation::{CandidateId, generation_manifest::decode_candidate};

use crate::x86_64_sandbox_boot_generation::{Compiled, Inputs, Shape, compile_uncached};

/// Cached Generations older than this are reclaimed, so the directory stays bounded.
const LIFETIME: Duration = Duration::from_hours(24 * 7);
/// The exact published Candidate bytes, from which identity and manifest are recovered.
const CANDIDATE: &str = "candidate.somacan";
/// The two normalized-rootfs facts the caller reports but the store does not carry.
const FACTS: &str = "facts.txt";

/// Lowercase hex of a finished digest; `Sha256` output does not implement `LowerHex`.
fn hex(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut out, byte| {
            use std::fmt::Write as _;
            write!(out, "{byte:02x}").expect("write to a String");
            out
        })
}

fn digest_file(path: &Path) -> String {
    let mut file = File::open(path).expect("open a cache key input");
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).expect("read a cache key input");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    hex(hasher)
}

/// Every input that can change the compiled bytes, and nothing that cannot.
///
/// The OCI layout is keyed by its index, which names the manifest digest, so a changed image
/// changes the key without reading a gigabyte of layers.
fn key(layout: &Path, reference: &str, shape: Shape, inputs: &Inputs) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"soma-generation-cache-v1\0");
    hasher.update(reference.as_bytes());
    hasher.update(b"\0");
    hasher.update(shape.memory_mib.to_le_bytes());
    hasher.update(shape.storage_mib.to_le_bytes());
    for path in [
        &layout.join("index.json"),
        &inputs.kernel,
        &inputs.kernel_config,
        &inputs.agent,
    ] {
        hasher.update(digest_file(path).as_bytes());
        hasher.update(b"\0");
    }
    hex(hasher)
}

fn reclaim_stale(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|since| since > LIFETIME));
        if stale {
            let _ignored = fs::remove_dir_all(entry.path());
        }
    }
}

/// Reopens a cached Generation, or `None` when the entry is absent or incomplete.
fn reopen(entry: &Path) -> Option<Compiled> {
    let bytes = fs::read(entry.join(CANDIDATE)).ok()?;
    let manifest = decode_candidate(&bytes).ok()?;
    let facts = fs::read_to_string(entry.join(FACTS)).ok()?;
    let (tree_digest, entry_count) = facts.split_once(' ')?;
    let store = entry.join("store");
    if !store.is_dir() {
        return None;
    }
    Some(Compiled {
        store,
        id: CandidateId::of(&bytes),
        manifest,
        tree_digest: tree_digest.to_owned(),
        entry_count: entry_count.trim().parse().ok()?,
    })
}

fn record(entry: &Path, compiled: &Compiled, candidate_bytes: &[u8]) {
    File::create(entry.join(CANDIDATE))
        .and_then(|mut file| file.write_all(candidate_bytes))
        .expect("record the cached Candidate bytes");
    File::create(entry.join(FACTS))
        .and_then(|mut file| write!(file, "{} {}", compiled.tree_digest, compiled.entry_count))
        .expect("record the cached rootfs facts");
}

/// Returns the cached Generation for these inputs, compiling it once if it is absent.
///
/// The build happens in a private directory and is renamed into place, so a torn build is
/// never observed as a hit and two concurrent runs cannot interleave into one entry.
pub fn compile(
    root: &Path,
    layout: &Path,
    reference: &str,
    shape: Shape,
    inputs: &Inputs,
) -> Compiled {
    fs::create_dir_all(root).expect("create the Generation cache root");
    reclaim_stale(root);
    let entry = root.join(key(layout, reference, shape, inputs));
    if let Some(hit) = reopen(&entry) {
        eprintln!(
            "[cache] reopened the compiled Generation at {}",
            entry.display()
        );
        return hit;
    }
    let building = root.join(format!(
        "building-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ignored = fs::remove_dir_all(&building);
    fs::create_dir_all(&building).expect("create the Generation build directory");
    let compiled = compile_uncached(layout, reference, shape, inputs, &building);
    let candidate_bytes =
        soma_generation::generation_manifest::encode_candidate(compiled.manifest())
            .expect("encode the compiled Candidate");
    record(&building, &compiled, &candidate_bytes);
    if fs::rename(&building, &entry).is_err() {
        // Another run published this key first. Its bytes are identical by construction, so
        // prefer the published entry and drop this build.
        let published = reopen(&entry);
        let _ignored = fs::remove_dir_all(&building);
        return published.expect("a competing run published an unreadable cache entry");
    }
    reopen(&entry).expect("reopen the Generation just published to the cache")
}
