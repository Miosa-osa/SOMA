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
//! `SOMA_PREPARE_VCPUS` and `SOMA_PREPARE_NETWORK` extend the shape past the one the flags
//! describe. A shape above one vCPU or three gigabytes targets compiler profile version 2, and
//! `SOMA_PREPARE_NETWORK=public_internet` declares the network device a Generation must carry
//! for the runner's large shape to put a leased egress bundle behind it. Without it a
//! Generation is built with no network device at all, and no later request can add one.
//!
//! Usage:
//!
//! ```text
//! prepare_generation <reference> <oci-layout> <kernel> <kernel-config> \
//!     <guest-agent> <erofs-tools> <e2fsprogs> <out-entry> [memory_mib] [storage_mib]
//! ```
//!
//! `SOMA_CAPTURE_WARM`, when set and non-empty, declares the Generation's capture warm plan:
//! commands separated by `;`, each an absolute executable followed by space-separated
//! arguments, for example `/usr/local/bin/node -v`. The plan is carried in the initramfs and
//! is therefore part of the Generation's identity. Unset, the Generation declares none.

use std::error::Error;
use std::path::PathBuf;

use soma::{Capabilities, MachineShape, OciImage};
use soma_generation::{
    CompilerProfile, LifetimeLimits, StartupBehavior, TemplateImage,
    TemplateRevision as CompilerRevision,
};
use soma_guest::{CaptureWarmPlan, WarmCommand};

#[path = "prepare_generation/build.rs"]
mod build;
#[path = "prepare_generation/publication.rs"]
mod publication;

use build::BuildInputs;

const DEFAULT_MEMORY_MIB: u64 = 1024;
const DEFAULT_STORAGE_MIB: u64 = 10240;
const DEFAULT_TTL_SECONDS: u64 = 3600;
/// The guest RAM above which the shape targets compiler profile version 2.
const V2_MEMORY_MIB: u64 = 3 * 1024;

struct Args {
    reference: String,
    inputs: BuildInputs,
    vcpus: u16,
    memory_mib: u64,
    storage_mib: u64,
    /// Whether this Generation declares a network device for a leased egress bundle.
    network: bool,
}

/// The shape and compiler profile this invocation builds.
///
/// A shape above one vCPU or three gigabytes is the version 2 machine, which is the same rule
/// the compiler's own tests apply, so a Generation built here is the machine its profile names.
fn shape(args: &Args) -> Result<(MachineShape, CompilerProfile), Box<dyn Error>> {
    let mut shape = MachineShape::new(args.vcpus, args.memory_mib, args.storage_mib)?;
    if args.network {
        shape = shape.with_capabilities(Capabilities::isolated().with_network_access());
    }
    let profile = if args.vcpus > 1 || args.memory_mib > V2_MEMORY_MIB {
        CompilerProfile::v2()
    } else {
        CompilerProfile::v1()
    };
    Ok((shape, profile))
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
    let vcpus = std::env::var("SOMA_PREPARE_VCPUS").map_or(Ok(1_u16), |value| {
        value
            .parse::<u16>()
            .map_err(|_| format!("SOMA_PREPARE_VCPUS must be a positive integer, got {value:?}"))
    })?;
    if vcpus == 0 {
        return Err("SOMA_PREPARE_VCPUS must be at least one".to_owned());
    }
    let network = match std::env::var("SOMA_PREPARE_NETWORK").as_deref() {
        Err(_) | Ok("" | "isolated") => false,
        Ok("public_internet") => true,
        Ok(other) => {
            return Err(format!(
                "SOMA_PREPARE_NETWORK must be isolated or public_internet, got {other:?}"
            ));
        }
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
        vcpus,
        network,
    })
}

/// The startup behavior, with the capture warm plan `SOMA_CAPTURE_WARM` declares, if any.
fn startup() -> Result<StartupBehavior, Box<dyn Error>> {
    let declared = std::env::var("SOMA_CAPTURE_WARM").unwrap_or_default();
    if declared.is_empty() {
        return Ok(StartupBehavior::readiness_only());
    }
    let commands = declared
        .split(';')
        .map(|command| WarmCommand::parse(command.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StartupBehavior::readiness_only().with_capture_warm(CaptureWarmPlan::new(commands)?))
}

fn run(args: &Args) -> Result<(), Box<dyn Error>> {
    let startup = startup()?;
    let (shape, profile) = shape(args)?;
    let policy_version = profile.policy_version;
    let prepared = build::prepare(&args.inputs, profile, |normalized, _store| {
        let workload = normalized.workload();
        Ok(CompilerRevision::new(
            TemplateImage::new(
                OciImage::parse(&args.reference)?,
                workload.manifest_digest().clone(),
                workload.platform().clone(),
            ),
            shape,
            startup.clone(),
            LifetimeLimits::new(DEFAULT_TTL_SECONDS)?,
            policy_version,
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
