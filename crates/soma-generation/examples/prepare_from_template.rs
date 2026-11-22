//! Compiles one Template document into a launchable Generation entry in a prepared store.
//!
//! This is `prepare_generation` with the Template pipeline in front of it. The document selects
//! the image, the command, the Machine shape, the network envelope, and the lifecycle; this tool
//! resolves it against the local OCI layout and the normalized rootfs, producing a Template Lock,
//! and compiles the Generation the Lock projects onto. The build inputs are unchanged: the same
//! kernel, guest agent, and filesystem tools, and the same prepared-store entry comes out.
//!
//! The order of work is import, then resolve. A Template cannot be resolved until something can
//! say whether the base image really carries its command's program, and only a normalized rootfs
//! can answer that, so the image is imported first and a rejected document costs one import.
//!
//! `SOMA_PREPARE_CEILING_MIB` declares the live-resize machine, as it does for `prepare_generation`:
//! the most guest memory the Generation can ever have, above the template's memory. It targets
//! compiler profile version 3 and the manifest records the hotplug region as the ceiling minus
//! the boot memory. Without a ceiling, the tool selects the first fixed-memory compiler profile
//! that can certify the Template's exact CPU, RAM, and writable-disk shape.
//!
//! Usage:
//!
//! ```text
//! prepare_from_template <template.toml> <oci-layout> <kernel> <kernel-config> \
//!     <guest-agent> <erofs-tools> <e2fsprogs> <out-entry>
//! ```

use std::error::Error;
use std::fs;
use std::path::PathBuf;

use soma_generation::{
    CompilerProfile, ImportLimits, LayoutResolver, ProfileLimits, RootfsOracle, compiler_revision,
    profile_v1_backend, profile_v2_backend,
};
use soma_template::{
    BackendCapabilities, ModuleRegistry, PolicyCeiling, ResourceLimits, Resources, Template,
    TemplateRevision as LockRevision, parse_template, resolve,
};

#[path = "prepare_generation/build.rs"]
mod build;
#[path = "prepare_generation/publication.rs"]
mod publication;

use build::BuildInputs;

const MIB: u64 = 1024 * 1024;
// Volume-enabled and live-resize profiles require explicit lifecycle inputs.
const FIXED_POLICIES: &[u16] = &[1, 2, 5, 6, 7, 8];
const LIVE_RESIZE_POLICIES: &[u16] = &[3];

struct Args {
    template: PathBuf,
    inputs: BuildInputs,
}

fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.len() != 8 {
        return Err(format!(
            "expected 8 arguments, got {}\n\
             usage: prepare_from_template <template.toml> <oci-layout> <kernel> <kernel-config> \
             <guest-agent> <erofs-tools> <e2fsprogs> <out-entry>",
            raw.len()
        ));
    }
    let ceiling_mib = build::ceiling_from_environment()?;
    Ok(Args {
        template: PathBuf::from(&raw[0]),
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
    })
}

fn compiler_profile(resources: &Resources, live_resize: bool) -> Result<CompilerProfile, String> {
    let storage_bytes = resources
        .writable_storage_mib
        .checked_mul(MIB)
        .ok_or_else(|| "template writable storage exceeds the supported size".to_owned())?;
    let policies = if live_resize {
        LIVE_RESIZE_POLICIES
    } else {
        FIXED_POLICIES
    };
    policies
        .iter()
        .filter_map(|&version| CompilerProfile::from_policy_version(version))
        .find(|profile| {
            let limits = ProfileLimits::for_version(profile.policy_version)
                .expect("implemented compiler profile has resource limits");
            (1..=u32::from(limits.max_vcpus)).contains(&resources.vcpus)
                && (128..=limits.max_memory_mib).contains(&resources.memory_mib)
                && (storage_bytes == 0 || profile.overlay_capacities.contains(&storage_bytes))
        })
        .ok_or_else(|| {
            format!(
                "no compiler profile supports {} vCPUs, {} MiB memory and {} MiB writable storage (live resize: {live_resize})",
                resources.vcpus, resources.memory_mib, resources.writable_storage_mib
            )
        })
}

fn backend_for(profile: &CompilerProfile) -> Result<BackendCapabilities, Box<dyn Error>> {
    match profile.policy_version {
        1 => Ok(profile_v1_backend()),
        2 | 3 => Ok(profile_v2_backend()),
        version => {
            let limits = ProfileLimits::for_version(version)
                .expect("implemented compiler profile has resource limits");
            let legacy = profile_v2_backend();
            Ok(BackendCapabilities::new(
                legacy.platforms(),
                &legacy.idle_actions(),
                ResourceLimits {
                    max_vcpus: u32::from(limits.max_vcpus),
                    max_memory_mib: limits.max_memory_mib,
                    max_writable_storage_mib: profile
                        .overlay_capacities
                        .iter()
                        .copied()
                        .max()
                        .unwrap_or(0)
                        / MIB,
                },
            )?)
        }
    }
}

fn run(args: &Args) -> Result<(), Box<dyn Error>> {
    let document = fs::read(&args.template)
        .map_err(|error| format!("{}: {error}", args.template.display()))?;
    let template: Template = parse_template(&document)?;
    // A ceiling makes this the live-resize machine (profile version 3); the build derives the
    // hotplug region from it and the template's boot memory.
    let live_resize = args.inputs.ceiling_mib.is_some();
    let base_profile = compiler_profile(template.resources(), live_resize)?;
    let version = base_profile.policy_version;
    let backend = backend_for(&base_profile)?;
    let prepared = build::prepare(&args.inputs, base_profile, |normalized, store| {
        let resolver = LayoutResolver::new(
            &args.inputs.layout,
            template.workload().image(),
            ImportLimits::default(),
        );
        let oracle = RootfsOracle::open(store, normalized, CompilerProfile::v1().tree)?;
        // The widest ceiling: this tool builds what the document asks for, and narrowing egress
        // is an organization policy decision that belongs to whoever runs the build, not here.
        let lock = resolve(
            &template,
            &resolver,
            &PolicyCeiling::unrestricted(),
            &backend,
            &oracle,
        )?;
        println!("lock: {}", lock.id());
        let view = LockRevision::from_lock(&lock)
            .with_provenance(&template, &ModuleRegistry::builtin())?;
        Ok(compiler_revision(&view, version)?)
    })?;
    build::report(&prepared, &args.inputs.out_entry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resources(vcpus: u32, memory_mib: u64, writable_storage_mib: u64) -> Resources {
        Resources {
            vcpus,
            memory_mib,
            writable_storage_mib,
        }
    }

    #[test]
    fn chooses_a_profile_that_can_certify_the_exact_template_shape() {
        for (shape, policy) in [
            (resources(1, 1024, 1024), 1),
            (resources(2, 4096, 0), 2),
            (resources(2, 4096, 10240), 8),
            (resources(16, 16384, 20480), 5),
            (resources(16, 32768, 20480), 6),
            (resources(16, 32768, 163840), 7),
        ] {
            assert_eq!(
                compiler_profile(&shape, false).unwrap().policy_version,
                policy
            );
        }
    }

    #[test]
    fn live_resize_uses_its_own_profile_and_refuses_incompatible_shapes() {
        assert_eq!(
            compiler_profile(&resources(2, 4096, 10240), true)
                .unwrap()
                .policy_version,
            3
        );
        assert!(compiler_profile(&resources(16, 32768, 163840), true).is_err());
        assert!(compiler_profile(&resources(2, 4096, 2048), false).is_err());
        assert!(compiler_profile(&resources(2, 4096, u64::MAX), false).is_err());
    }
}

fn main() {
    if soma_version::version_requested(std::env::args_os()) {
        print!(
            "{}",
            soma_version::report(
                "prepare_from_template",
                &soma_version::build!(),
                &soma_version::Speaks::default(),
            )
        );
        return;
    }
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
