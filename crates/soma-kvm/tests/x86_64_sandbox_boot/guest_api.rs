//! The guest API v2 proof: one real guest agent answers the level probe and the level 2 file
//! operations, and a command sees the persisted environment, the command context, and output
//! as it is produced.
//!
//! Run with `SOMA_GUEST_AGENT` set to the agent under test and `SOMA_EXPECT_GUEST_API` set to
//! `2` for the current agent or `1` for the agent that predates the API. The same test binary
//! runs both, so the old-guest half of the compatibility matrix is a real old agent and not a
//! simulation.

#![allow(
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::elidable_lifetime_names,
    reason = "a live proof is one long scenario read top to bottom"
)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};
use soma_generation::open_artifact;
use soma_guest::{
    CommandContext, EntryKind, FileFailure, FileOutcome, FileRequest, GuestApi, GuestCommand,
    OperationId, OutputSource, RangeRead, RangeWrite, RepairedHostControl, WholeFileRead,
    WholeFileWrite,
};
use soma_kvm::MachineContract;

use crate::{
    x86_64_discover::kernel_path,
    x86_64_sandbox_boot_control::HostIo,
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{require_kvm, require_scratch_space, scratch_dir},
    x86_64_sandbox_boot_session as session,
};

const MIB: u64 = 1024 * 1024;
const PART: usize = 4 << 20;
const TOTAL: usize = 20 << 20;
type Repaired<'a> = RepairedHostControl<HostIo<'a>>;

fn expected_level() -> u8 {
    std::env::var("SOMA_EXPECT_GUEST_API")
        .ok()
        .and_then(|value| value.parse().ok())
        .expect("set SOMA_EXPECT_GUEST_API to 1 or 2")
}

fn operation() -> OperationId {
    OperationId::new(session::random16()).unwrap()
}

fn nonce() -> Vec<u8> {
    session::random16()
        .iter()
        .flat_map(|byte| format!("{byte:02x}").into_bytes())
        .collect()
}

/// Deterministic bytes that are not periodic at any size a transfer chunk could hide behind.
fn pattern(length: usize) -> Vec<u8> {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

fn hex(digest: &[u8]) -> String {
    digest.iter().fold(String::new(), |mut text, byte| {
        write!(text, "{byte:02x}").unwrap();
        text
    })
}

fn file<'a>(
    repaired: Repaired<'a>,
    request: FileRequest,
) -> Result<(Repaired<'a>, FileOutcome), String> {
    repaired
        .file(request)
        .map_err(|error| format!("file request: {error}"))
}

fn path(text: &str) -> Box<[u8]> {
    text.as_bytes().into()
}

fn run<'a>(
    repaired: Repaired<'a>,
    arguments: &[&str],
    context: CommandContext,
) -> Result<(Repaired<'a>, String, String), String> {
    let command = GuestCommand::new(
        b"/bin/busybox".to_vec(),
        arguments
            .iter()
            .map(|argument| argument.as_bytes().to_vec())
            .collect(),
        30_000,
        1 << 20,
    )
    .and_then(|command| command.with_context(context))
    .map_err(|error| format!("command: {error}"))?;
    let (repaired, outcome) = repaired
        .execute(operation(), command)
        .map_err(|error| format!("execute: {error}"))?;
    eprintln!("[guest-api] run {arguments:?} -> {:?}", outcome.status());
    Ok((
        repaired,
        String::from_utf8_lossy(outcome.stdout()).into_owned(),
        String::from_utf8_lossy(outcome.stderr()).into_owned(),
    ))
}

macro_rules! check {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(format!($($message)+));
        }
    };
}

/// Everything the proof asserts, in the order a host would use the API.
#[path = "guest_api_scenario.rs"]
mod scenario;

#[path = "guest_api_context.rs"]
mod context;

#[path = "guest_api_files.rs"]
mod files;

/// The virtual directory the agent answers the probe under.
fn crate_probe_prefix() -> Vec<u8> {
    soma_guest::GUEST_API_PROBE_PREFIX.to_vec()
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and the busybox OCI layout"]
fn the_guest_api_behaves_as_its_level_says() {
    let _serialized = crate::live::serialize_live_proof();
    require_kvm();
    require_scratch_space();
    let name = "guest-api";
    let scratch = scratch_dir(name);
    let kernel = kernel_path();
    let inputs = generation::inputs(kernel);
    let layout = generation::oci_layout(crate::live::BUSYBOX, "SOMA_OCI_BUSYBOX_LAYOUT", &scratch)
        .expect("the busybox OCI layout could not be exported; set SOMA_OCI_BUSYBOX_LAYOUT");
    let shape = generation::Shape::new(1024, 2048, 2);
    let compiled = generation::compile(
        &layout,
        &format!("docker.io/library/{}", crate::live::BUSYBOX),
        shape,
        &inputs,
        &scratch,
    );
    let manifest = &compiled.manifest();
    eprintln!(
        "[{name}] expecting guest API level {} with agent digest {}",
        expected_level(),
        manifest.guest_agent.descriptor.digest
    );
    let kernel = open_artifact(&compiled.store, &manifest.kernel.descriptor).unwrap();
    let initramfs = open_artifact(&compiled.store, &manifest.initramfs.descriptor).unwrap();
    let root = open_artifact(&compiled.store, &manifest.root.descriptor).unwrap();
    let mut template =
        open_artifact(&compiled.store, &manifest.overlay.templates[0].descriptor).unwrap();
    let head = generation::private_head(&mut template, &scratch.join("overlay-head.ext4"));
    drop(template);
    let config = session::config(
        kernel,
        initramfs,
        root,
        head,
        session::Machine {
            ram_bytes: shape.memory_mib * MIB,
            vcpus: shape.vcpus,
            contract: MachineContract::require(manifest.machine_contract.version)
                .expect("the compiled Generation names a contract this host builds"),
        },
        manifest.device_set(),
    );
    let (evidence, outcome) =
        session::run_scenario(config, compiled.id().as_str(), scenario::scenario);
    session::evidence::report(name, &evidence, &scratch.join("serial.log"));
    if let Err(error) = outcome {
        panic!(
            "[{name}] the scenario failed: {error}; exit={:?}",
            evidence.exit
        );
    }
    eprintln!("[{name}] PASS at guest API level {}", expected_level());
}
