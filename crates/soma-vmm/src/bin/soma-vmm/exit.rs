//! The worker's exit status contract.
//!
//! Every status below is a decision the worker made about its own situation. Anything else a
//! supervisor observes came from the kernel or the jail: `SIGSYS` is the seccomp filter,
//! `SIGKILL` is the cgroup or the launcher, and a status the worker never uses means the
//! process died before it could choose.

/// The lifecycle ended the way it was asked to.
pub const OK: i32 = 0;
/// The worker was started without a decodable descriptor manifest.
pub const NO_MANIFEST: i32 = 2;
/// The manifest named no reachable control descriptor, so nothing can be reported.
pub const NO_CONTROL: i32 = 3;
/// The attestation did not describe a jail, so no request was served.
pub const UNCONTAINED: i32 = 4;
/// The supervisor sent more requests than one worker life admits.
pub const REQUEST_BUDGET: i32 = 5;
/// The control socket reached end of stream, so nobody is left to serve.
///
/// This is a distinct status rather than a clean one, because losing a supervisor is not the
/// ordered end of a lifecycle and must not be recorded as one.
pub const SUPERVISOR_GONE: i32 = 7;
/// The guest shut the machine down from inside. The sandbox chooses it and exits with it, as
/// `soma_vmm::sandbox::guest_end::GUEST_POWER_OFF_STATUS`.
pub const GUEST_POWER_OFF: i32 = 8;
/// The guest restarted the machine from inside, as `GUEST_REBOOT_STATUS`.
pub const GUEST_REBOOT: i32 = 9;
/// The build target is not one this worker runs on.
pub const UNSUPPORTED: i32 = 6;
