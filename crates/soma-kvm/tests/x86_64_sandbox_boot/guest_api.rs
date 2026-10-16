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
fn scenario<'a>(repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    let want = expected_level();
    let nonce = nonce();

    // The level probe, then a file planted at the probe path by a workload.
    let (repaired, api) = repaired
        .probe_guest_api(&nonce)
        .map_err(|error| format!("probe: {error}"))?;
    eprintln!("[guest-api] probe -> {api:?}");
    check!(
        api == if want >= 2 {
            GuestApi::Level(2)
        } else {
            GuestApi::Legacy
        },
        "probe answered {api:?}, expected level {want}"
    );
    // A workload plants files at the probe path, one with the shape of a genuine answer but a
    // nonce it could not have known. None may change what the host believes.
    let (repaired, made) = file(
        repaired,
        FileRequest::MakeDirectory {
            path: path("/.soma/guest-api"),
            parents: true,
        },
    )?;
    eprintln!("[guest-api] mkdir /.soma/guest-api -> {made:?}");
    check!(
        made == FileOutcome::Done,
        "could not create the probe directory: {made:?}"
    );
    let planted = [crate_probe_prefix(), nonce.clone()].concat();
    let mut repaired = repaired;
    for forged in [&b"2\n"[..], b"2 0123456789abcdef\n", b"2"] {
        let (next, written) = repaired
            .write_whole_file(&planted, forged, 64)
            .map_err(|error| format!("plant: {error}"))?;
        repaired = next;
        eprintln!("[guest-api] plant {forged:?} at the probe path -> {written:?}");
        check!(
            written == WholeFileWrite::Written,
            "the plant was not written: {written:?}"
        );
        let (next, seen) = repaired
            .probe_guest_api(&nonce)
            .map_err(|error| format!("probe after plant: {error}"))?;
        repaired = next;
        eprintln!("[guest-api] probe after plant -> {seen:?}");
        check!(
            seen == api,
            "a planted file changed the answer: {api:?} -> {seen:?}"
        );
    }
    if want < 2 {
        eprintln!(
            "[guest-api] level 1: the host must not send a level 2 request; nothing more is asked"
        );
        // The old guest still does everything it did before: a whole write, a read, a listing.
        let (repaired, written) = repaired
            .write_whole_file(b"/tmp/legacy.txt", b"hello", 64)
            .map_err(|error| format!("legacy write: {error}"))?;
        check!(
            written == WholeFileWrite::Written,
            "legacy write: {written:?}"
        );
        let (repaired, read) = repaired
            .read_whole_file(b"/tmp/legacy.txt", 64)
            .map_err(|error| format!("legacy read: {error}"))?;
        check!(
            read == WholeFileRead::Bytes(b"hello".to_vec()),
            "legacy read: {read:?}"
        );
        let (repaired, listed) = file(
            repaired,
            FileRequest::ReadDirectory {
                path: path("/tmp"),
                offset: 0,
            },
        )?;
        check!(
            matches!(&listed, FileOutcome::Listed { entries, .. } if entries.iter().any(|entry| &*entry.name == b"legacy.txt")),
            "legacy listing: {listed:?}"
        );
        return Ok(repaired);
    }

    // Level 2 files: a 20 MiB ranged write and read, compared by digest on both sides.
    let data = pattern(TOTAL);
    let expected = hex(&Sha256::digest(&data));
    let mut repaired = repaired;
    for (index, chunk) in data.chunks(PART).enumerate() {
        let (next, outcome) = repaired
            .write_range(
                RangeWrite {
                    path: b"/tmp/big.bin",
                    offset: (index * PART) as u64,
                    create: index == 0,
                    shorten: (index + 1) * PART >= TOTAL,
                    bytes: chunk,
                },
                PART,
            )
            .map_err(|error| format!("write_range {index}: {error}"))?;
        repaired = next;
        check!(
            outcome == WholeFileWrite::Written,
            "write_range {index}: {outcome:?}"
        );
    }
    let (next, stat) = file(
        repaired,
        FileRequest::Stat {
            path: path("/tmp/big.bin"),
        },
    )?;
    repaired = next;
    eprintln!("[guest-api] stat big.bin -> {stat:?}");
    check!(
        matches!(stat, FileOutcome::Stat { kind: EntryKind::File, size, .. } if size == TOTAL as u64),
        "stat: {stat:?}"
    );
    let mut read_back = Vec::with_capacity(TOTAL);
    let mut offset = 0_u64;
    loop {
        let (next, read) = repaired
            .read_range(b"/tmp/big.bin", offset, PART, PART)
            .map_err(|error| format!("read_range at {offset}: {error}"))?;
        repaired = next;
        let RangeRead::Bytes { bytes, end } = read else {
            return Err(format!("read_range at {offset}: {read:?}"));
        };
        offset += bytes.len() as u64;
        read_back.extend_from_slice(&bytes);
        if end || bytes.is_empty() {
            break;
        }
    }
    let observed = hex(&Sha256::digest(&read_back));
    eprintln!(
        "[guest-api] sha256 host-sent={expected} host-read-back={observed} bytes={}",
        read_back.len()
    );
    check!(
        read_back.len() == TOTAL && observed == expected,
        "the 20 MiB round trip differs"
    );
    let (next, guest_sum, _) = run(
        repaired,
        &["sha256sum", "/tmp/big.bin"],
        CommandContext::default(),
    )?;
    repaired = next;
    eprintln!("[guest-api] sha256 guest-side={}", guest_sum.trim());
    check!(
        guest_sum.starts_with(&expected),
        "the guest computed a different digest: {guest_sum}"
    );

    // Rename replaces atomically; chmod; stat reports both.
    let (next, outcome) = file(
        repaired,
        FileRequest::Rename {
            path: path("/tmp/big.bin"),
            to: path("/tmp/renamed.bin"),
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "rename: {outcome:?}");
    let (next, outcome) = file(
        repaired,
        FileRequest::SetMode {
            path: path("/tmp/renamed.bin"),
            mode: 0o640,
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "chmod: {outcome:?}");
    let (next, stat) = file(
        repaired,
        FileRequest::Stat {
            path: path("/tmp/renamed.bin"),
        },
    )?;
    repaired = next;
    check!(
        matches!(stat, FileOutcome::Stat { size, mode, .. } if size == TOTAL as u64 && mode == 0o640),
        "stat after rename and chmod: {stat:?}"
    );
    let (next, gone) = file(
        repaired,
        FileRequest::Stat {
            path: path("/tmp/big.bin"),
        },
    )?;
    repaired = next;
    check!(
        gone == FileOutcome::Failed(FileFailure::NotFound),
        "the source remained: {gone:?}"
    );

    // A detailed listing reports sizes and a long directory pages by offset.
    let (next, listing) = file(
        repaired,
        FileRequest::ReadDirectoryDetailed {
            path: path("/tmp"),
            offset: 0,
        },
    )?;
    repaired = next;
    eprintln!("[guest-api] detailed listing -> {listing:?}");
    check!(
        matches!(&listing, FileOutcome::ListedDetailed { entries, .. }
            if entries.iter().any(|entry| &*entry.name == b"renamed.bin" && entry.size == TOTAL as u64)),
        "detailed listing lacks renamed.bin with its size"
    );

    // Persisted environment: written the way the control plane writes it (temporary file, mode
    // 0600, rename), then seen by a direct command with no restart; a command's own value wins.
    let (next, _) = file(
        repaired,
        FileRequest::MakeDirectory {
            path: path("/etc/miosa"),
            parents: true,
        },
    )?;
    repaired = next;
    let env_file = b"FROM_FILE='from file'\nOVERRIDDEN='file value'\n";
    let (next, outcome) = repaired
        .write_range(
            RangeWrite {
                path: b"/etc/miosa/.identity.env.tmp",
                offset: 0,
                create: true,
                shorten: true,
                bytes: env_file,
            },
            4096,
        )
        .map_err(|error| format!("env temp: {error}"))?;
    repaired = next;
    check!(outcome == WholeFileWrite::Written, "env temp: {outcome:?}");
    let (next, outcome) = file(
        repaired,
        FileRequest::SetMode {
            path: path("/etc/miosa/.identity.env.tmp"),
            mode: 0o600,
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "env chmod: {outcome:?}");
    let (next, outcome) = file(
        repaired,
        FileRequest::Rename {
            path: path("/etc/miosa/.identity.env.tmp"),
            to: path("/etc/miosa/identity.env"),
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "env rename: {outcome:?}");
    let context = CommandContext {
        environment: vec![(b"OVERRIDDEN".to_vec(), b"command value".to_vec())],
        working_directory: Some(b"/tmp".to_vec()),
        ..CommandContext::default()
    };
    let (next, env_out, _) = run(repaired, &["env"], context)?;
    repaired = next;
    eprintln!(
        "[guest-api] env sees: {}",
        env_out
            .lines()
            .filter(|line| line.contains("FILE") || line.contains("OVERRIDDEN"))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    check!(
        env_out.lines().any(|line| line == "FROM_FILE=from file"),
        "the persisted variable is missing"
    );
    check!(
        env_out
            .lines()
            .any(|line| line == "OVERRIDDEN=command value"),
        "the command's own value did not win"
    );
    check!(
        !env_out.contains("OVERRIDDEN=file value"),
        "the file value leaked past the command's"
    );

    // Update without a restart: the next command sees the new value.
    let (next, outcome) = repaired
        .write_range(
            RangeWrite {
                path: b"/etc/miosa/identity.env",
                offset: 0,
                create: false,
                shorten: true,
                bytes: b"FROM_FILE='updated'\n",
            },
            4096,
        )
        .map_err(|error| format!("env update: {error}"))?;
    repaired = next;
    check!(
        outcome == WholeFileWrite::Written,
        "env update: {outcome:?}"
    );
    let (next, env_out, _) = run(repaired, &["env"], CommandContext::default())?;
    repaired = next;
    check!(
        env_out.lines().any(|line| line == "FROM_FILE=updated"),
        "an update needed a restart"
    );

    // Working directory and stdin from the command context.
    let context = CommandContext {
        working_directory: Some(b"/tmp".to_vec()),
        stdin: b"typed input".to_vec(),
        ..CommandContext::default()
    };
    let (next, out, _) = run(repaired, &["sh", "-c", "pwd; cat"], context)?;
    repaired = next;
    check!(out == "/tmp\ntyped input", "cwd and stdin: {out:?}");

    // Output arrives while the command runs, not when it ends.
    let command = GuestCommand::new(
        b"/bin/busybox".to_vec(),
        vec![
            b"sh".to_vec(),
            b"-c".to_vec(),
            b"echo first; sleep 2; echo second".to_vec(),
        ],
        30_000,
        1 << 20,
    )
    .map_err(|error| format!("streaming command: {error}"))?;
    let started = Instant::now();
    let mut seen: Vec<(Duration, String)> = Vec::new();
    let (next, outcome) = repaired
        .execute_streaming(operation(), command, |source, bytes| {
            if source == OutputSource::Stdout {
                seen.push((
                    started.elapsed(),
                    String::from_utf8_lossy(bytes).into_owned(),
                ));
            }
        })
        .map_err(|error| format!("execute_streaming: {error}"))?;
    repaired = next;
    eprintln!(
        "[guest-api] streamed chunks (elapsed, text): {seen:?}; final {:?}",
        outcome.status()
    );
    let first = seen
        .iter()
        .find(|(_, text)| text.contains("first"))
        .map(|(at, _)| *at);
    let second = seen
        .iter()
        .find(|(_, text)| text.contains("second"))
        .map(|(at, _)| *at);
    check!(
        matches!((first, second), (Some(first), Some(second)) if second.saturating_sub(first) >= Duration::from_millis(1500)),
        "output was not incremental: {seen:?}"
    );
    Ok(repaired)
}

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
    let (evidence, outcome) = session::run_scenario(config, compiled.id().as_str(), scenario);
    session::evidence::report(name, &evidence, &scratch.join("serial.log"));
    if let Err(error) = outcome {
        panic!(
            "[{name}] the scenario failed: {error}; exit={:?}",
            evidence.exit
        );
    }
    eprintln!("[{name}] PASS at guest API level {}", expected_level());
}
