//! Drives one sandbox from cold boot through the authenticated session to cleanup and prints
//! the evidence table.

use std::{
    fs::{self, File},
    io::Read as _,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use soma_guest::{
    GuestCommand, HostControl, HostLaunchMaterial, LaunchNetwork, OperationId, ResponderKeypair,
    TerminalStatus,
};
use soma_kvm::x86_64::{
    DeviceIdentity, GuestExit, Milestone, SandboxConfig, SandboxDisks, SandboxEvidence,
    SandboxMachine,
};

use crate::x86_64_sandbox_boot_control::HostIo;

const PAGE_DOMAIN: &[u8] = b"SOMA-LAUNCH-PAGE";
const GUEST_CID: u32 = 3;
const GUEST_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];
pub const BOOT_DEADLINE: Duration = Duration::from_secs(60);
const EXIT_GRACE: Duration = Duration::from_secs(10);

/// What the session must do once the guest is Ready.
pub struct Command<'a> {
    pub program: &'a [u8],
    pub arguments: &'a [&'a [u8]],
    pub timeout_millis: u32,
    pub output_bytes: u64,
}

/// The authenticated result of the one command.
pub struct Executed {
    pub status: TerminalStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn random16() -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .expect("read fresh identity bytes");
    bytes
}

fn generation_bytes(id: &str) -> [u8; 32] {
    let hex = id.strip_prefix("sha256:").expect("GenerationId prefix");
    let mut bytes = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks(2).enumerate() {
        bytes[index] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    bytes
}

fn now_unix_nanos() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )
    .unwrap()
}

pub fn responder_key(keypair: &ResponderKeypair) -> [u8; 32] {
    keypair.private_key().expose_for_provisioning(|key| *key)
}

/// Boots `config`, completes the session with `responder`, executes `command`, shuts down,
/// and cleans up.
///
/// Returns the evidence together with the command result, or the evidence and the failure.
pub fn run_with_responder(
    config: SandboxConfig,
    generation_id: &str,
    command: &Command<'_>,
    responder: &ResponderKeypair,
) -> (SandboxEvidence, Result<Executed, String>) {
    let network = LaunchNetwork::new(
        GUEST_CID,
        1,
        GUEST_MAC,
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        now_unix_nanos(),
    )
    .expect("link-down placeholder network");
    let material = HostLaunchMaterial::generate(
        generation_bytes(generation_id),
        random16(),
        random16(),
        network,
    )
    .expect("fresh launch material");
    let mut sandbox = match SandboxMachine::create(config) {
        Ok(sandbox) => sandbox,
        Err(error) => panic!("sandbox creation failed: {error}"),
    };
    let outcome = drive(&mut sandbox, material, responder, command);
    let evidence = sandbox.finish(EXIT_GRACE);
    (evidence, outcome)
}

fn drive(
    sandbox: &mut SandboxMachine,
    material: HostLaunchMaterial,
    responder: &ResponderKeypair,
    command: &Command<'_>,
) -> Result<Executed, String> {
    let delivered = material
        .deliver_with(|page| sandbox.write_launch_page(page))
        .map_err(|error| format!("launch page delivery: {error}"))?;
    sandbox.start().map_err(|error| format!("start: {error}"))?;
    let boot_deadline = Instant::now() + BOOT_DEADLINE;
    sandbox
        .wait_launch_page_consumed(PAGE_DOMAIN, boot_deadline)
        .map_err(|error| format!("launch page consumption: {error}"))?;
    sandbox
        .control()
        .wait_connected(boot_deadline)
        .map_err(|error| format!("vsock connection: {error}"))?;
    sandbox.mark(Milestone::VsockConnected);
    let host = HostControl::connect(delivered, responder.public_key(), HostIo::new(sandbox))
        .map_err(|error| format!("handshake: {error}"))?;
    sandbox.mark(Milestone::Handshake);
    let repaired = host
        .prepare_and_probe()
        .map_err(|error| format!("repair and probe: {error}"))?;
    sandbox.mark(Milestone::Ready);
    let guest_command = GuestCommand::new(
        command.program.to_vec(),
        command.arguments.iter().map(|arg| arg.to_vec()).collect(),
        command.timeout_millis,
        command.output_bytes,
    )
    .map_err(|error| format!("command: {error}"))?;
    let (repaired, outcome) = repaired
        .execute(OperationId::new(random16()).unwrap(), guest_command)
        .map_err(|error| format!("execute: {error}"))?;
    sandbox.mark(Milestone::Execute);
    let executed = Executed {
        status: outcome.status(),
        stdout: outcome.stdout().to_vec(),
        stderr: outcome.stderr().to_vec(),
    };
    repaired
        .shutdown(OperationId::new(random16()).unwrap())
        .map_err(|error| format!("shutdown: {error}"))?;
    sandbox.mark(Milestone::Shutdown);
    Ok(executed)
}

/// Assembles the sandbox inputs from opened artifacts.
pub fn config(
    kernel: File,
    initramfs: File,
    root: File,
    overlay: File,
    ram_bytes: u64,
) -> SandboxConfig {
    SandboxConfig {
        kernel,
        initramfs,
        disks: SandboxDisks { root, overlay },
        identity: DeviceIdentity {
            guest_cid: GUEST_CID,
            guest_mac: GUEST_MAC,
        },
        ram_bytes,
    }
}

/// Prints the timeline, phases, counters, and the console tail; retains the console log.
pub fn report(label: &str, evidence: &SandboxEvidence, log: &Path) {
    fs::write(log, &evidence.serial).unwrap();
    let text = String::from_utf8_lossy(&evidence.serial);
    let lines: Vec<&str> = text.lines().collect();
    eprintln!(
        "[{label}] serial log ({} bytes, {} lines) retained at {}",
        evidence.serial.len(),
        lines.len(),
        log.display()
    );
    for line in lines.iter().rev().take(16).rev() {
        eprintln!("  | {line}");
    }
    eprintln!("[{label}] COLD timeline (ns since sandbox creation began; delta from previous):");
    let mut previous = 0;
    for mark in &evidence.timeline {
        eprintln!(
            "  {:<20} {:>14} {:>+14}",
            format!("{:?}", mark.milestone),
            mark.elapsed_ns,
            i128::from(mark.elapsed_ns) - i128::from(previous)
        );
        previous = mark.elapsed_ns;
    }
    for timing in &evidence.phases {
        eprintln!(
            "  phase={:?} elapsed_ns={}",
            timing.phase(),
            timing.elapsed_ns()
        );
    }
    eprintln!(
        "[{label}] cmdline={:?} entry={:#x} initramfs={:?} exit={:?} launch_page_retired={}",
        evidence.cmdline,
        evidence.entry,
        evidence.initramfs,
        evidence.exit,
        evidence.launch_page_retired
    );
    eprintln!(
        "[{label}] bus={:?} uart={:?} mmio={:?}",
        evidence.bus, evidence.uart, evidence.mmio
    );
    eprintln!("[{label}] devices={:?}", evidence.devices);
}

/// The assertions every successful sandbox run must satisfy.
pub fn assert_orderly(evidence: &SandboxEvidence) {
    assert_eq!(
        evidence.exit,
        Ok(GuestExit::Reset),
        "guest did not stop orderly"
    );
    assert!(evidence.launch_page_retired, "launch page was not retired");
    for milestone in [
        Milestone::RunStart,
        Milestone::KernelInit,
        Milestone::LaunchPageConsumed,
        Milestone::VsockConnected,
        Milestone::Handshake,
        Milestone::LaunchPageRetired,
        Milestone::Ready,
        Milestone::AgentReadyLine,
        Milestone::Execute,
        Milestone::Shutdown,
        Milestone::GuestExit,
        Milestone::Cleanup,
    ] {
        assert!(
            evidence.at(milestone).is_some(),
            "milestone {milestone:?} missing"
        );
    }
    assert!(
        evidence.devices.first_fault.is_none(),
        "a device faulted: {:?}",
        evidence.devices
    );
    assert_eq!(evidence.mmio.transport_violations, 0, "{:?}", evidence.mmio);
    assert_eq!(evidence.mmio.notify_exits, 0, "{:?}", evidence.mmio);
    assert_eq!((evidence.bus.other_in, evidence.bus.other_out), (0, 0));
    let text = String::from_utf8_lossy(&evidence.serial);
    assert!(text.contains("soma-guest-agent: ready"));
    assert!(text.contains("soma-guest-agent: shutdown acknowledged"));
    assert!(!text.contains("poisoned by"));
}
