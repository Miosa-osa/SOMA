//! Captures one prepared entry's snapshot so later launches restore instead of cold booting.
//!
//! Cold booting a Generation costs hundreds of milliseconds on the request path. A snapshot taken
//! once, at the guest agent's disconnected repair point, lets every later launch resume a machine
//! that is already through kernel boot and userspace init.
//!
//! The source machine is booted with **no launch page**, so it reaches the repair point with no
//! Instance identity, no session, and no key anywhere in guest memory. That is what makes the
//! captured object safe to share across every Instance restored from it.
//!
//! The snapshot is written to `<entry>/snapshot/`, beside the `store/` the Candidate describes.
//!
//! Usage:
//!
//! ```text
//! capture_snapshot <prepared-entry> [memory_mib]
//! ```

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::path::PathBuf;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::time::Duration;

// The example target root resolves child modules under `examples/`, so the two halves of this
// program are named by explicit path rather than by the usual sibling-directory rule.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "capture_snapshot/capture.rs"]
mod capture;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "capture_snapshot/publish.rs"]
mod publish;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use capture::run;

/// The console line the pinned agent prints when it parks awaiting launch material.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const REPAIR_POINT_LINE: &[u8] = b"soma-guest-agent: awaiting launch material";
/// The context identifier the source machine holds; every restore is given a fresh one.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const CAPTURE_CID: u32 = 3;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const GUEST_MAC: [u8; 6] = [0x02, 0x53, 0x4f, 0x4d, 0x41, 0x01];
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const MIB: u64 = 1024 * 1024;
/// How long the source machine has to announce its repair point.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const REPAIR_POINT_DEADLINE: Duration = Duration::from_secs(120);
/// How long the vCPU has to leave `KVM_RUN` once it is kicked.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PAUSE_GRACE: Duration = Duration::from_secs(10);

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() || arguments.len() > 2 {
        eprintln!("usage: capture_snapshot <prepared-entry> [memory_mib]");
        std::process::exit(2);
    }
    let entry = PathBuf::from(&arguments[0]);
    let memory_mib = arguments
        .get(1)
        .map_or(Ok(1024), |value| value.parse::<u64>())
        .unwrap_or(1024);
    if let Err(error) = run(&entry, memory_mib) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn main() {
    eprintln!("capture_snapshot requires Linux x86_64 with KVM");
    std::process::exit(2);
}
