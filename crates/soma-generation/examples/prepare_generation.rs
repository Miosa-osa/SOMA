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
//! Usage:
//!
//! ```text
//! prepare_generation <reference> <oci-layout> <kernel> <kernel-config> \
//!     <guest-agent> <erofs-tools> <e2fsprogs> <out-entry> [memory_mib] [storage_mib]
//! ```

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use soma::{MachineShape, OciImage, OciPlatform};
use soma_generation::{
    BuildHost, CompileGeneration, CompilerProfile, ImportLimits, ImportOciLayout, LifetimeLimits,
    MachineInputs, NormalizeOciRootfs, OciSelection, RootfsLimits, StartupBehavior, TemplateImage,
    TemplateRevision, Toolchain, compile_generation, generation_manifest::encode_candidate,
    import_oci_layout, normalize_oci_rootfs,
};

const MIB: u64 = 1024 * 1024;
const DEFAULT_MEMORY_MIB: u64 = 1024;
const DEFAULT_STORAGE_MIB: u64 = 10240;

struct Args {
    reference: String,
    layout: PathBuf,
    kernel: PathBuf,
    kernel_config: PathBuf,
    agent: PathBuf,
    erofs_tools: PathBuf,
    e2fsprogs: PathBuf,
    out_entry: PathBuf,
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
        layout: PathBuf::from(&raw[1]),
        kernel: PathBuf::from(&raw[2]),
        kernel_config: PathBuf::from(&raw[3]),
        agent: PathBuf::from(&raw[4]),
        erofs_tools: PathBuf::from(&raw[5]),
        e2fsprogs: PathBuf::from(&raw[6]),
        out_entry: PathBuf::from(&raw[7]),
        memory_mib: raw
            .get(8)
            .map_or(Ok(DEFAULT_MEMORY_MIB), |v| number(v, "memory_mib"))?,
        storage_mib: raw
            .get(9)
            .map_or(Ok(DEFAULT_STORAGE_MIB), |v| number(v, "storage_mib"))?,
    })
}

fn require_present(path: &Path, kind: &str, directory: bool) -> Result<(), String> {
    let present = if directory {
        path.is_dir()
    } else {
        path.is_file()
    };
    if present {
        Ok(())
    } else {
        Err(format!("{kind} not found at {}", path.display()))
    }
}

fn run(args: &Args) -> Result<(), Box<dyn Error>> {
    require_present(&args.layout.join("oci-layout"), "OCI layout", false)?;
    require_present(&args.kernel, "kernel", false)?;
    require_present(&args.kernel_config, "kernel configuration", false)?;
    require_present(&args.agent, "guest agent", false)?;
    require_present(&args.erofs_tools, "erofs tools directory", true)?;
    require_present(&args.e2fsprogs, "e2fsprogs directory", true)?;

    // A fresh entry each time, so a re-prepared reference cannot mix old and new artifacts.
    if args.out_entry.exists() {
        fs::remove_dir_all(&args.out_entry)?;
    }
    let store = args.out_entry.join("store");
    let staging = args.out_entry.join("staging");
    fs::create_dir_all(&store)?;
    fs::create_dir_all(&staging)?;

    let platform = OciPlatform::new("linux", "amd64", None)?;
    let imported = import_oci_layout(ImportOciLayout::new(
        &args.layout,
        &store,
        OciSelection::Platform(&platform),
        ImportLimits::default(),
    ))?;
    let normalized = normalize_oci_rootfs(NormalizeOciRootfs::new(
        &imported,
        &store,
        RootfsLimits::default(),
    ))?;

    let workload = normalized.workload();
    let template = TemplateRevision::new(
        TemplateImage::new(
            OciImage::parse(&args.reference)?,
            workload.manifest_digest().clone(),
            workload.platform().clone(),
        ),
        MachineShape::new(1, args.memory_mib, args.storage_mib)?,
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(3600)?,
        1,
    )?;

    let mut profile = CompilerProfile::v1();
    profile.overlay_capacities = vec![args.storage_mib * MIB];

    let compiled = compile_generation(CompileGeneration::new(
        &template,
        &normalized,
        &store,
        &profile,
        BuildHost::new(
            &staging,
            Toolchain::new(&args.erofs_tools, &args.e2fsprogs),
            MachineInputs::new(&args.kernel, &args.kernel_config, &args.agent, &args.agent),
        ),
    ))?;

    // The entry is what a prepared store holds: the published Candidate bytes, the artifact store
    // those bytes describe, and the reference this entry answers to.
    let candidate_bytes = encode_candidate(&compiled.candidate.manifest)?;
    fs::write(args.out_entry.join("candidate.somacan"), &candidate_bytes)?;
    fs::write(args.out_entry.join("reference"), args.reference.as_bytes())?;
    // Staging held only intermediate build files; the store is self-contained.
    let _ignored = fs::remove_dir_all(&staging);

    println!(
        "prepared {} at {}\n  candidate id: {}\n  entries: {}",
        args.reference,
        args.out_entry.display(),
        compiled.candidate.id.as_str(),
        normalized.entry_count(),
    );
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
