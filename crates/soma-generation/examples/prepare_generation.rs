//! Compiles one OCI image into a launchable Generation entry in a prepared store.
//!
//! This is the primitive the server-setup runbook needs and the harness previously kept to
//! itself: it imports an OCI layout, normalizes the rootfs, and runs the Generation compiler with
//! the pinned kernel, its configuration, the static guest agent, and the filesystem tools, then
//! writes one prepared-store entry that the KVM backend can resolve and launch.
//!
//! A prepared-store entry is a directory holding `store/` (the artifact store the compiler wrote),
//! `candidate.somacan` (the exact published Candidate bytes), and `reference` (the image the entry
//! was prepared for). Point the backend at the parent directory with `SOMA_GENERATION_STORE`.
//!
//! The Machine shape comes from the command line here. `prepare_from_template` runs the same
//! pipeline with the shape, lifetime, and network envelope taken from a Template document.
//!
//! Usage:
//!
//! ```text
//! prepare_generation <reference> <oci-layout> <kernel> <kernel-config> \
//!     <guest-agent> <erofs-tools> <e2fsprogs> <out-entry> [memory_mib] [storage_mib]
//! ```

use std::error::Error;
use std::path::PathBuf;

use soma::{MachineShape, OciImage};
use soma_generation::{
    LifetimeLimits, StartupBehavior, TemplateImage, TemplateRevision as CompilerRevision,
};

#[path = "prepare_generation/build.rs"]
mod build;
#[path = "prepare_generation/publication.rs"]
mod publication;

use build::BuildInputs;

const DEFAULT_MEMORY_MIB: u64 = 1024;
const DEFAULT_STORAGE_MIB: u64 = 10240;
const DEFAULT_TTL_SECONDS: u64 = 3600;

struct Args {
    reference: String,
    inputs: BuildInputs,
    memory_mib: u64,
    storage_mib: u64,
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.len() < 8 || raw.len() > 10 {
        return Err(format!(
            "expected 8 to 10 arguments, got {}\n\
             usage: prepare_generation <reference> <oci-layout> <kernel> <kernel-config> \
             <guest-agent> <erofs-tools> <e2fsprogs> <out-entry> [memory_mib] [storage_mib]",
            raw.len()
        ));
    }
    let number = |value: &str, name: &str| {
        value
            .parse::<u64>()
            .map_err(|_| format!("{name} must be a positive integer, got {value:?}"))
    };
    Ok(Args {
        reference: raw[0].clone(),
        inputs: BuildInputs {
            layout: PathBuf::from(&raw[1]),
            kernel: PathBuf::from(&raw[2]),
            kernel_config: PathBuf::from(&raw[3]),
            agent: PathBuf::from(&raw[4]),
            erofs_tools: PathBuf::from(&raw[5]),
            e2fsprogs: PathBuf::from(&raw[6]),
            out_entry: PathBuf::from(&raw[7]),
        },
        memory_mib: raw
            .get(8)
            .map_or(Ok(DEFAULT_MEMORY_MIB), |v| number(v, "memory_mib"))?,
        storage_mib: raw
            .get(9)
            .map_or(Ok(DEFAULT_STORAGE_MIB), |v| number(v, "storage_mib"))?,
    })
}

fn run(args: &Args) -> Result<(), Box<dyn Error>> {
    let prepared = build::prepare(&args.inputs, |normalized, _store| {
        let workload = normalized.workload();
        Ok(CompilerRevision::new(
            TemplateImage::new(
                OciImage::parse(&args.reference)?,
                workload.manifest_digest().clone(),
                workload.platform().clone(),
            ),
            MachineShape::new(1, args.memory_mib, args.storage_mib)?,
            StartupBehavior::readiness_only(),
            LifetimeLimits::new(DEFAULT_TTL_SECONDS)?,
            1,
        )?)
    })?;
    build::report(&prepared, &args.inputs.out_entry);
    Ok(())
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if let Err(error) = run(&args) {
        eprintln!("prepare failed: {error}");
        std::process::exit(1);
    }
}
