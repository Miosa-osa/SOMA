#![allow(dead_code)]

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use soma::{MachineShape, OciImage};
use soma_generation::{
    BuildHost, CompileGeneration, CompiledGeneration, CompilerProfile, LifetimeLimits,
    MachineInputs, NormalizedRootfs, StartupBehavior, TemplateImage, TemplateRevision, Toolchain,
    compile_generation,
};

const MIB: u64 = 1024 * 1024;

/// Locates the pinned erofs-utils directory through `SOMA_EROFS_TOOLS` or `PATH`.
pub fn erofs_tools() -> Option<PathBuf> {
    if let Some(directory) = env::var_os("SOMA_EROFS_TOOLS") {
        let directory = PathBuf::from(directory);
        return directory.join("mkfs.erofs").is_file().then_some(directory);
    }
    find_on_path("mkfs.erofs")
}

/// Locates the e2fsprogs directory through `SOMA_E2FSPROGS`, `PATH`, or the sbin defaults.
pub fn e2fsprogs() -> Option<PathBuf> {
    if let Some(directory) = env::var_os("SOMA_E2FSPROGS") {
        let directory = PathBuf::from(directory);
        return directory.join("mke2fs").is_file().then_some(directory);
    }
    find_on_path("mke2fs").or_else(|| {
        ["/usr/sbin", "/sbin"]
            .into_iter()
            .map(PathBuf::from)
            .find(|directory| directory.join("mke2fs").is_file())
    })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?).find(|directory| directory.join(name).is_file())
}

/// Prints an explicit skip line and returns `None` when a toolchain is absent.
pub fn toolchains(test: &str) -> Option<(PathBuf, PathBuf)> {
    let erofs = erofs_tools();
    let e2fs = e2fsprogs();
    match (erofs, e2fs) {
        (Some(erofs), Some(e2fs)) => Some((erofs, e2fs)),
        (erofs, e2fs) => {
            eprintln!(
                "SKIP {test}: pinned toolchain not found (mkfs.erofs found={}, mke2fs found={}); \
                 set SOMA_EROFS_TOOLS and SOMA_E2FSPROGS or add them to PATH",
                erofs.is_some(),
                e2fs.is_some()
            );
            None
        }
    }
}

/// A synthetic `x86_64` `ET_EXEC` ELF with one executable load segment and an optional PVH note.
pub fn synthetic_kernel(note_entry: Option<u32>, load_paddr: u64) -> Vec<u8> {
    let mut notes = Vec::new();
    if let Some(entry) = note_entry {
        notes.extend_from_slice(&4_u32.to_le_bytes());
        notes.extend_from_slice(&4_u32.to_le_bytes());
        notes.extend_from_slice(&18_u32.to_le_bytes());
        notes.extend_from_slice(b"Xen\0");
        notes.extend_from_slice(&entry.to_le_bytes());
    }
    let code = vec![0x90_u8; 64];
    let header_len = 64_u64;
    let ph_len = 56_u64 * 2;
    let code_offset = header_len + ph_len;
    let notes_offset = code_offset + code.len() as u64;
    let mut elf = Vec::new();
    elf.extend_from_slice(b"\x7fELF");
    elf.extend_from_slice(&[2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    elf.extend_from_slice(&2_u16.to_le_bytes());
    elf.extend_from_slice(&0x3e_u16.to_le_bytes());
    elf.extend_from_slice(&1_u32.to_le_bytes());
    elf.extend_from_slice(&load_paddr.to_le_bytes());
    elf.extend_from_slice(&header_len.to_le_bytes());
    elf.extend_from_slice(&0_u64.to_le_bytes());
    elf.extend_from_slice(&0_u32.to_le_bytes());
    elf.extend_from_slice(&64_u16.to_le_bytes());
    elf.extend_from_slice(&56_u16.to_le_bytes());
    elf.extend_from_slice(&2_u16.to_le_bytes());
    elf.extend_from_slice(&64_u16.to_le_bytes());
    elf.extend_from_slice(&0_u16.to_le_bytes());
    elf.extend_from_slice(&0_u16.to_le_bytes());
    assert_eq!(elf.len(), 64);
    program_header(
        &mut elf,
        [1, 5],
        [code_offset, load_paddr, code.len() as u64, 4096],
    );
    program_header(
        &mut elf,
        [4, 4],
        [notes_offset, 0, notes.len() as u64, notes.len() as u64],
    );
    elf.extend_from_slice(&code);
    elf.extend_from_slice(&notes);
    elf
}

fn program_header(elf: &mut Vec<u8>, kind_flags: [u32; 2], layout: [u64; 4]) {
    let [kind, flags] = kind_flags;
    let [offset, paddr, file_size, memory_size] = layout;
    elf.extend_from_slice(&kind.to_le_bytes());
    elf.extend_from_slice(&flags.to_le_bytes());
    elf.extend_from_slice(&offset.to_le_bytes());
    elf.extend_from_slice(&paddr.to_le_bytes());
    elf.extend_from_slice(&paddr.to_le_bytes());
    elf.extend_from_slice(&file_size.to_le_bytes());
    elf.extend_from_slice(&memory_size.to_le_bytes());
    elf.extend_from_slice(&4096_u64.to_le_bytes());
}

/// A kernel configuration text satisfying profile v1.
pub fn kernel_config() -> String {
    let mut text =
        String::from("# Generated test configuration\nCONFIG_PHYSICAL_START=0x1000000\n");
    for symbol in soma_generation::kernel_config::REQUIRED_BUILTIN {
        text.push_str(symbol);
        text.push_str("=y\n");
    }
    text.push_str("# CONFIG_MODULES is not set\nCONFIG_PCI=n\n");
    text
}

/// A small compiler profile for tests: two writable classes that stay cheap to hash.
pub fn test_profile() -> CompilerProfile {
    let mut profile = CompilerProfile::v1();
    profile.overlay_capacities = vec![64 * MIB, 128 * MIB];
    profile
}

/// Writes the five machine inputs into `directory` and returns their paths.
pub fn write_machine_inputs(directory: &Path, agent: &[u8]) -> [PathBuf; 5] {
    fs::create_dir_all(directory).unwrap();
    let paths = [
        directory.join("vmlinux"),
        directory.join("kernel.config"),
        directory.join("init"),
        directory.join("soma-guest-agent"),
        directory.join("responder.key"),
    ];
    fs::write(&paths[0], synthetic_kernel(Some(0x0100_0010), 0x0100_0000)).unwrap();
    fs::write(&paths[1], kernel_config()).unwrap();
    fs::write(&paths[2], b"#!/bin/sh\nexec /bin/soma-guest-agent\n").unwrap();
    fs::write(&paths[3], agent).unwrap();
    fs::write(&paths[4], b"synthetic-responder-private-key!").unwrap();
    paths
}

/// A Template revision that selects the normalized tree, 256 MiB of memory, the 64 MiB
/// writable class, the isolated network policy, readiness only, and the given lifetime.
pub fn test_template(normalized: &NormalizedRootfs, ttl_seconds: u64) -> TemplateRevision {
    let workload = normalized.workload();
    TemplateRevision::new(
        TemplateImage::new(
            OciImage::parse("example.test/fixture:amd64").unwrap(),
            workload.manifest_digest().clone(),
            workload.platform().clone(),
        ),
        MachineShape::new(1, 256, 64).unwrap(),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(ttl_seconds).unwrap(),
        1,
    )
    .unwrap()
}

/// Compiles one Generation with the test profile and synthetic machine inputs.
pub fn compile(
    normalized: &NormalizedRootfs,
    store: &Path,
    scratch: &Path,
    tools: &(PathBuf, PathBuf),
    agent: &[u8],
) -> Result<CompiledGeneration, soma_generation::CompileError> {
    compile_with_template(
        &test_template(normalized, 3600),
        normalized,
        store,
        scratch,
        tools,
        agent,
    )
}

/// Compiles one Generation for an explicit Template revision.
pub fn compile_with_template(
    template: &TemplateRevision,
    normalized: &NormalizedRootfs,
    store: &Path,
    scratch: &Path,
    tools: &(PathBuf, PathBuf),
    agent: &[u8],
) -> Result<CompiledGeneration, soma_generation::CompileError> {
    let staging = scratch.join("staging");
    fs::create_dir_all(&staging).unwrap();
    let inputs = write_machine_inputs(&scratch.join("inputs"), agent);
    let profile = test_profile();
    compile_generation(CompileGeneration::new(
        template,
        normalized,
        store,
        &profile,
        BuildHost::new(
            &staging,
            Toolchain::new(&tools.0, &tools.1),
            MachineInputs::new(&inputs[0], &inputs[1], &inputs[2], &inputs[3], &inputs[4]),
        ),
    ))
}
