//! Executing the Generation's capture warm plan once, before the snapshot capture point.
//!
//! Reading a runtime binary (see `warm`) pages in its text but not what running it maps: the
//! dynamic linker, its shared libraries, and the interpreter's own startup data. Measured on a
//! `node:22` Generation, a restored first `node -v` still paid tens of milliseconds for that.
//! A Generation may therefore declare a few commands the agent executes once at the
//! disconnected repair point. Their file pages stay in the guest page cache, the capture records
//! them, and every restored Instance finds them resident.
//!
//! The plan comes from the initramfs, whose digest the manifest binds, and is read before the
//! agent leaves the initramfs root. It executes before any launch material exists: there is no
//! Instance identity, secret, entropy seed, time sample, network, or session in the guest to
//! observe or capture. Each command is also confined so that the only thing it can leave behind
//! is page cache:
//!
//! - it runs in its own session, and its whole process group and then every other process in
//!   the guest are killed and reaped afterwards, so no process survives into the snapshot;
//! - it runs in private mount, IPC, and UTS namespaces in which the root, `/dev`, `/dev/pts`,
//!   `/proc`, and `/sys` are read-only, so it cannot write a file, a sysctl, a System V IPC
//!   object, or a hostname that every Instance would then share;
//! - it runs as the unprivileged `nobody` account with no supplementary groups and
//!   `no_new_privs`, so it cannot undo any of that;
//! - it gets an empty environment apart from a fixed `PATH`, `/` as its directory, and the null
//!   device for every standard stream.
//!
//! Warming is advisory. A command that is missing, fails, or overruns its budget is reported and
//! killed, and the machine still reaches the repair point; only a declaration that cannot be
//! read or decoded fails the boot, because the verified initramfs cannot legitimately hold one.

#![allow(unsafe_code)]

use std::fs::File;
use std::io::{self, Read as _};
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use soma_guest::{CaptureWarmPlan, MAX_WARM_PLAN_BYTES};

use crate::descendants;

/// Where the plan sits in the initramfs root, which is the agent's root until early init
/// switches into the Generation root.
pub const PLAN_PATH: &str = "/warm";
/// Wall-clock budget of one warm command.
pub const COMMAND_BUDGET: Duration = Duration::from_secs(10);
/// The unprivileged account warm commands run as.
pub const NOBODY: libc::uid_t = 65_534;

/// Quiet time after the plan and before the capture.
///
/// Exiting commands and their torn-down namespaces leave deferred kernel work behind. Captured
/// unfinished, it runs in every restored Instance on the ready path: measured on `node:22`, the
/// first restore of a Generation captured without this wait reached ready in about 47 ms instead
/// of 22 ms. Waiting here costs only capture time.
const SETTLE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(1);
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// Mounts made read-only inside each command's private mount namespace, parents first.
const READ_ONLY: [&std::ffi::CStr; 5] = [c"/", c"/dev", c"/dev/pts", c"/proc", c"/sys"];

/// Why the declared plan could not be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclarationFault {
    /// The file exists but could not be read.
    Unreadable,
    /// The bytes are not a canonical plan within bounds.
    Invalid,
}

/// What executing one plan did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Outcome {
    /// Commands that exited with status zero.
    pub succeeded: usize,
    /// Commands that could not start, failed, or overran their budget.
    pub failed: usize,
    /// Processes killed and reaped after the plan, beyond each command's own leader.
    pub swept: usize,
}

/// Reads the plan the initramfs declares, if any.
///
/// # Errors
///
/// Returns a fault for a present plan that cannot be read or is not canonical.
pub fn declared() -> Result<Option<CaptureWarmPlan>, DeclarationFault> {
    read_plan(PLAN_PATH)
}

fn read_plan(path: &str) -> Result<Option<CaptureWarmPlan>, DeclarationFault> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DeclarationFault::Unreadable),
    };
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_WARM_PLAN_BYTES).unwrap_or(u64::MAX) + 1;
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| DeclarationFault::Unreadable)?;
    CaptureWarmPlan::decode(&bytes)
        .map(Some)
        .map_err(|_| DeclarationFault::Invalid)
}

/// Executes every command of the plan in order, each confined and bounded, and leaves no
/// process behind.
#[must_use]
pub fn execute(plan: &CaptureWarmPlan) -> Outcome {
    let mut outcome = Outcome::default();
    for command in plan.commands() {
        if run_one(command.executable(), command.arguments(), COMMAND_BUDGET) {
            outcome.succeeded += 1;
        } else {
            outcome.failed += 1;
        }
    }
    outcome.swept = descendants::sweep_strays();
    thread::sleep(SETTLE);
    outcome
}

/// Runs one confined command to completion or its deadline; returns whether it exited zero.
fn run_one(executable: &str, arguments: &[String], budget: Duration) -> bool {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .env("PATH", PATH)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the closure runs in the forked child before `execve` and calls only
    // async-signal-safe system calls on static, NUL-terminated strings; it allocates nothing
    // and touches no lock the parent may have held.
    unsafe { command.pre_exec(confine) };
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let group = i32::try_from(child.id()).unwrap_or(0);
    let deadline = Instant::now() + budget;
    let succeeded = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            Ok(None) | Err(_) => {
                descendants::kill_group(group);
                let _ = child.wait();
                break false;
            }
        }
    };
    descendants::kill_group(group);
    descendants::reap_group(group);
    succeeded
}

/// Confines the forked child; any failure aborts the spawn so nothing runs unconfined.
fn confine() -> io::Result<()> {
    // SAFETY: each call below is a plain system call on process-local state or on static
    // NUL-terminated strings with no memory preconditions beyond valid pointers, which the
    // `CStr` constants and null pointers provide.
    unsafe {
        check(libc::setsid())?;
        check(libc::unshare(
            libc::CLONE_NEWNS | libc::CLONE_NEWIPC | libc::CLONE_NEWUTS,
        ))?;
        // Stop the read-only remounts below from propagating back to the agent's namespace.
        check(libc::mount(
            std::ptr::null(),
            c"/".as_ptr(),
            std::ptr::null(),
            libc::MS_REC | libc::MS_PRIVATE,
            std::ptr::null(),
        ))?;
        for target in READ_ONLY {
            check(libc::mount(
                std::ptr::null(),
                target.as_ptr(),
                std::ptr::null(),
                libc::MS_REMOUNT | libc::MS_BIND | libc::MS_RDONLY,
                std::ptr::null(),
            ))?;
        }
        check(libc::setgroups(0, std::ptr::null()))?;
        check(libc::setresgid(NOBODY, NOBODY, NOBODY))?;
        check(libc::setresuid(NOBODY, NOBODY, NOBODY))?;
        check(libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0))?;
    }
    Ok(())
}

fn check(result: libc::c_int) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
