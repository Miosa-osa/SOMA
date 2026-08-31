//! Every Template shipped in `templates/` must parse and resolve.
//!
//! These are the documents a first-time author copies, so a broken one is worse than a
//! missing one: it teaches the wrong shape and it fails at the point where the author has
//! the least context to debug it. The test walks the directory rather than naming files, so
//! a new example is covered the moment it is committed.

use std::{fs, path::PathBuf};

use soma::{OciDigest, OciImage, OciPlatform};
use soma_template::{
    BackendCapabilities, EgressIntent, FilesystemOracle, IdleAction, IngressIntent, OciResolver,
    OracleError, PolicyCeiling, ResolveError, ResolvedImage, ResourceLimits, TemplateLock,
    parse_template, resolve,
};

/// Pins any reference to one fixed digest.
///
/// Resolution against a real registry is a separate slice; what this test proves is that the
/// document is well formed and that validation admits it, not which bytes a registry would
/// serve today.
struct FixedResolver;

const DIGEST: &str = "sha256:9c1185a5c5e9fc54612808977ee8f548b2258d31ee2c8a2a0e4a7b0d5b2f1c3d";

impl OciResolver for FixedResolver {
    fn resolve(
        &self,
        _reference: &OciImage,
        platform: &OciPlatform,
    ) -> Result<ResolvedImage, ResolveError> {
        let digest = OciDigest::parse(DIGEST).expect("fixture digest is canonical");
        Ok(ResolvedImage::new(digest, platform.clone(), 1_234))
    }
}

/// Answers for the executables a stock Linux base image carries.
///
/// Nothing in the workspace can inspect a base image filesystem yet, so the set is stated
/// here instead of discovered. It is deliberately short: an example whose command is not one
/// of these must get its program from a module, which is the shape we want examples to teach.
struct BaseImageOracle;

const PRESENT: &[&str] = &["/bin/sh", "/bin/bash", "/usr/local/bin/python3"];

impl FilesystemOracle for BaseImageOracle {
    fn executable_present(
        &self,
        _image: &ResolvedImage,
        program: &str,
    ) -> Result<bool, OracleError> {
        Ok(PRESENT.iter().any(|path| {
            *path == program || (!program.contains('/') && path.rsplit('/').next() == Some(program))
        }))
    }
}

fn ceiling() -> PolicyCeiling {
    PolicyCeiling::new(EgressIntent::Allowlist, IngressIntent::Deny)
        .with_domains(&["api.anthropic.com", "github.com"])
        .expect("bounded ceiling")
}

fn backend() -> BackendCapabilities {
    BackendCapabilities::new(
        &[OciPlatform::linux_amd64(), OciPlatform::linux_arm64()],
        &[IdleAction::Destroy, IdleAction::Stop],
        ResourceLimits {
            max_vcpus: 8,
            max_memory_mib: 16_384,
            max_writable_storage_mib: 65_536,
        },
    )
    .expect("bounded backend")
}

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("templates")
}

fn documents() -> Vec<(String, Vec<u8>)> {
    let mut documents = Vec::new();
    for entry in fs::read_dir(directory()).expect("templates directory is present") {
        let path = entry.expect("directory entry is readable").path();
        if path.extension().is_none_or(|extension| extension != "toml") {
            continue;
        }
        let name = path
            .file_name()
            .expect("a file with an extension has a name")
            .to_string_lossy()
            .into_owned();
        documents.push((name, fs::read(&path).expect("example is readable")));
    }
    documents.sort_by(|left, right| left.0.cmp(&right.0));
    documents
}

fn lock(bytes: &[u8]) -> TemplateLock {
    let template = parse_template(bytes).expect("example parses");
    resolve(
        &template,
        &FixedResolver,
        &ceiling(),
        &backend(),
        &BaseImageOracle,
    )
    .expect("example resolves")
}

#[test]
fn the_examples_directory_is_not_empty() {
    let documents = documents();
    assert!(
        documents.len() >= 2,
        "templates/ must ship a minimal and a realistic example, found {}",
        documents.len()
    );
}

#[test]
fn every_example_parses_and_resolves() {
    for (name, bytes) in documents() {
        let template = match parse_template(&bytes) {
            Ok(template) => template,
            Err(error) => panic!("templates/{name} does not parse: {error}"),
        };
        match resolve(
            &template,
            &FixedResolver,
            &ceiling(),
            &backend(),
            &BaseImageOracle,
        ) {
            Ok(_) => {}
            Err(error) => panic!("templates/{name} does not validate: {error}"),
        }
    }
}

#[test]
fn every_example_locks_to_a_distinct_identity() {
    let mut identities: Vec<String> = documents()
        .iter()
        .map(|(_, bytes)| lock(bytes).id().to_string())
        .collect();
    let count = identities.len();
    identities.sort();
    identities.dedup();
    assert_eq!(
        identities.len(),
        count,
        "two examples select the same inputs"
    );
}
