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
    CompilerProfile, ImportLimits, LayoutResolver, RootfsOracle, compiler_revision,
    profile_v1_backend,
};
use soma_template::{
    ModuleRegistry, PolicyCeiling, Template, TemplateRevision as LockRevision, parse_template,
    resolve,
};

#[path = "prepare_generation/build.rs"]
mod build;
#[path = "prepare_generation/publication.rs"]
mod publication;

use build::BuildInputs;

/// The compiler profile the produced Generation targets.
const PROFILE_VERSION: u16 = 1;

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
        },
    })
}

fn run(args: &Args) -> Result<(), Box<dyn Error>> {
    let document = fs::read(&args.template)
        .map_err(|error| format!("{}: {error}", args.template.display()))?;
    let template: Template = parse_template(&document)?;
    let prepared = build::prepare(&args.inputs, |normalized, store| {
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
            &profile_v1_backend(),
            &oracle,
        )?;
        println!("lock: {}", lock.id());
        let view = LockRevision::from_lock(&lock)
            .with_provenance(&template, &ModuleRegistry::builtin())?;
        Ok(compiler_revision(&view, PROFILE_VERSION)?)
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
