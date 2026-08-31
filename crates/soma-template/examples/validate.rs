//! Validate one Template document and print the Template Lock identity it would receive.
//!
//! This lives here rather than behind a `soma template` subcommand on purpose. Resolving a
//! Template needs two external inputs the workspace does not have yet: an OCI resolver that
//! asks a registry for the exact manifest digest of a reference, and a filesystem oracle that
//! can say whether the base image actually carries the command's program. Both are stubbed
//! below. Shipping those stubs inside the `soma` binary would mean the product printing a
//! lock identity it did not really resolve, so the stub-driven pipeline stays an example
//! until a registry client exists, and the subcommand can then be a thin wrapper over it.
//!
//! Usage: `cargo run -p soma-template --example validate -- <template.toml> [<image-digest>]`
//!
//! Without a digest the document is parsed and reported. With one the digest is treated as
//! the pinned manifest, the Template is resolved, and the resulting `LockId` is printed.

use std::{fs, process};

use soma::{OciDigest, OciImage, OciPlatform};
use soma_template::{
    BackendCapabilities, FilesystemOracle, IdleAction, OciResolver, OracleError, PolicyCeiling,
    ResolveError, ResolvedImage, ResourceLimits, Template, parse_template, resolve,
};

fn main() {
    let mut arguments = std::env::args().skip(1);
    let Some(path) = arguments.next() else {
        eprintln!("usage: validate <template.toml> [<image-digest>]");
        process::exit(64);
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => fail(&format!("{path}: {error}")),
    };
    let template = match parse_template(&bytes) {
        Ok(template) => template,
        Err(error) => fail(&format!("{path}: {error}")),
    };
    report(&template);
    let Some(digest) = arguments.next() else {
        println!("lock: not computed; pass a pinned image digest to resolve one");
        return;
    };
    let digest = match OciDigest::parse(&digest) {
        Ok(digest) => digest,
        Err(error) => fail(&format!("{digest}: {error}")),
    };
    match resolve(
        &template,
        &Pinned { digest },
        &PolicyCeiling::unrestricted(),
        &backend(),
        &Present,
    ) {
        Ok(lock) => println!("lock: {}", lock.id()),
        Err(error) => fail(&format!("{path}: {error}")),
    }
}

fn report(template: &Template) {
    println!("name: {}", template.name());
    if let Some(description) = template.description() {
        println!("description: {description}");
    }
    println!("image: {}", template.workload().image().as_str());
    let platform = template.workload().platform();
    println!(
        "platform: {}/{}",
        platform.operating_system(),
        platform.architecture()
    );
    for module in template.modules() {
        println!("module: {module}");
    }
    match template.command() {
        Some(command) => println!("command: {} {:?}", command.program(), command.args()),
        None => println!("command: from the single module default"),
    }
    let resources = template.resources();
    println!(
        "resources: {} vcpu, {} MiB memory, {} MiB writable storage",
        resources.vcpus, resources.memory_mib, resources.writable_storage_mib
    );
    println!(
        "network: egress {:?}, ingress {:?}",
        template.network().egress,
        template.network().ingress
    );
}

fn fail(detail: &str) -> ! {
    eprintln!("validate: {detail}");
    process::exit(65);
}

/// Answers every reference with the digest the caller pinned on the command line.
struct Pinned {
    digest: OciDigest,
}

impl OciResolver for Pinned {
    fn resolve(
        &self,
        _reference: &OciImage,
        platform: &OciPlatform,
    ) -> Result<ResolvedImage, ResolveError> {
        Ok(ResolvedImage::new(self.digest.clone(), platform.clone(), 0))
    }
}

/// Assumes the base image carries whatever program the command names.
///
/// Nothing here can open the image, so the alternative is refusing every Template whose
/// program comes from the base image rather than from a module. The assumption is stated so
/// that a reader never mistakes this for a real check.
struct Present;

impl FilesystemOracle for Present {
    fn executable_present(
        &self,
        _image: &ResolvedImage,
        _program: &str,
    ) -> Result<bool, OracleError> {
        Ok(true)
    }
}

/// A Backend wide enough that the ceiling and the Backend never hide an authoring mistake.
fn backend() -> BackendCapabilities {
    BackendCapabilities::new(
        &[OciPlatform::linux_amd64(), OciPlatform::linux_arm64()],
        &[
            IdleAction::Destroy,
            IdleAction::Stop,
            IdleAction::Checkpoint,
        ],
        ResourceLimits {
            max_vcpus: 64,
            max_memory_mib: 262_144,
            max_writable_storage_mib: 1_048_576,
        },
    )
    .expect("bounded backend")
}
