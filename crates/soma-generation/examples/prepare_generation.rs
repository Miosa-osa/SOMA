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
//! `SOMA_PREPARE_CEILING_MIB` declares the live-resize machine: the most guest memory the
//! Generation can ever have. It targets compiler profile version 3, whose manifest records the
//! hotplug region as the ceiling minus the boot memory, and it must be above the boot memory.
//! Without it the Generation is built with no memory device and cannot be resized in memory.
//!
//! `SOMA_PREPARE_PERSISTENCE=1` declares the durable data volume: it targets compiler profile
//! version 4, whose manifest carries the persistence group and the class the volume is created
//! at, so a capture of this Generation declares the device and every launch of it binds a file
//! of the same class. Nothing about a shape implies it, so it is a setting rather than a rule.
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
/// A ceiling makes it the live-resize machine, and `SOMA_PREPARE_PERSISTENCE` makes it the
/// persistent one, because a Generation that declares a durable volume is not a machine a shape
/// implies: it is a policy a caller has to ask for.
fn shape(args: &Args) -> Result<(MachineShape, CompilerProfile), Box<dyn Error>> {
    let mut shape = MachineShape::new(args.vcpus, args.memory_mib, args.storage_mib)?;
    if args.network {
        shape = shape.with_capabilities(Capabilities::isolated().with_network_access());
    }
    // A ceiling makes this the live-resize machine; the build derives its hotplug region from it.
    let profile = if persistence_requested()? {
        CompilerProfile::v4()
    } else if args.inputs.ceiling_mib.is_some() {
        CompilerProfile::v3()
    } else if args.vcpus > 1 || args.memory_mib > V2_MEMORY_MIB {
        CompilerProfile::v2()
    } else {
        CompilerProfile::v1()
    };
    Ok((shape, profile))
}

/// Whether this invocation builds the persistent policy, from `SOMA_PREPARE_PERSISTENCE`.
///
/// It is a setting rather than a shape rule because nothing about a machine implies a durable
/// volume: the caller has to say that this Generation carries one, and the class it carries is
/// the profile's own, so a capture and every launch of it agree without being told again.
pub(crate) fn persistence_requested() -> Result<bool, String> {
    persistence_from(std::env::var("SOMA_PREPARE_PERSISTENCE").as_deref().ok())
}

/// The same reading of one configured value, so the rule is testable without the environment.
pub(crate) fn persistence_from(configured: Option<&str>) -> Result<bool, String> {
    match configured {
        None | Some("" | "0" | "false") => Ok(false),
        Some("1" | "true") => Ok(true),
        Some(other) => Err(format!(
            "SOMA_PREPARE_PERSISTENCE must be 1 or 0, got {other:?}"
        )),
    }
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
    let ceiling_mib = build::ceiling_from_environment()?;
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
            ceiling_mib,
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

#[cfg(test)]
mod tests {
    use super::persistence_from;

    #[test]
    fn the_persistence_policy_is_asked_for_rather_than_implied() {
        // Absent, blank, and the two false spellings are all "no"; the two true spellings are
        // "yes"; anything else is refused rather than read as one of them.
        for value in [None, Some(""), Some("0"), Some("false")] {
            assert_eq!(persistence_from(value), Ok(false), "value {value:?}");
        }
        for value in [Some("1"), Some("true")] {
            assert_eq!(persistence_from(value), Ok(true), "value {value:?}");
        }
        assert!(persistence_from(Some("yes")).is_err());
    }
}
