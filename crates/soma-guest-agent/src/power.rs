//! Shut down and restart from inside the guest.
//!
//! The agent is init, and the kernel delivers an init only the signals init installed handlers
//! for. The init shutdown convention is the one `poweroff`, `reboot` and `halt` from
//! sysvinit and busybox already speak: `SIGUSR2` powers off, `SIGTERM` restarts, `SIGUSR1`
//! halts, and `SIGINT` is the keyboard's ctrl-alt-del. Without handlers a guest had no way to end
//! its own machine: a user's Shut down was lost, and the machine kept running.
//!
//! Every request ends the machine in a way the host observes: processes are asked to exit, then
//! killed, and filesystems are flushed. Every request then ends the machine through the
//! keyboard controller's reset line (`pid1::poweroff`): the guest kernel has no power-off without
//! ACPI, and its lockdown refuses port access. A power-off or halt prints
//! [`soma_guest::POWER_OFF_LINE`] on the console first, which the VMM reads at the reset as a
//! guest shutdown; without it the reset is a restart, and the host starts the same Instance again.
//!
//! A handler only writes the signal's number to a pipe, which is async-signal-safe; a thread
//! reads it and does the work. Handlers are reset on `exec`, so no process the agent starts
//! inherits them.

#![allow(unsafe_code)]

use std::{
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

use crate::{console, descendants, pid1};

/// The signals a guest shutdown or restart arrives as.
const REQUESTS: [i32; 4] = [libc::SIGUSR2, libc::SIGTERM, libc::SIGUSR1, libc::SIGINT];

/// How long processes are given to exit on their own before they are killed.
const GRACE: Duration = Duration::from_secs(3);

/// The pipe's write end, for the handler; `-1` until the watch is installed.
static WRITE_END: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_request(signal: libc::c_int) {
    let fd = WRITE_END.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = u8::try_from(signal).unwrap_or(0);
        // SAFETY: `write` is async-signal-safe and `byte` is a valid one-byte buffer; a full
        // pipe drops the byte, which is harmless because one request already ends the machine.
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
}

/// What one received signal asks for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    PowerOff,
    Restart,
    Halt,
    CtrlAltDel,
}

impl Request {
    #[must_use]
    pub const fn from_signal(signal: i32) -> Option<Self> {
        match signal {
            libc::SIGUSR2 => Some(Self::PowerOff),
            libc::SIGTERM => Some(Self::Restart),
            libc::SIGUSR1 => Some(Self::Halt),
            libc::SIGINT => Some(Self::CtrlAltDel),
            _ => None,
        }
    }
}

/// Installs the handlers and returns the read end the requests arrive on.
///
/// # Errors
///
/// Returns the `errno` of the pipe or a handler that could not be installed.
pub fn install() -> Result<i32, i32> {
    let mut fds = [0; 2];
    // SAFETY: `pipe2` writes two descriptors into the valid two-element array.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 {
        return Err(errno());
    }
    WRITE_END.store(fds[1], Ordering::Relaxed);
    for signal in REQUESTS {
        // SAFETY: a zeroed `sigaction` is a valid empty action, and the handler is a plain
        // `extern "C"` function that only touches an atomic and calls `write`.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        let handler: extern "C" fn(libc::c_int) = on_request;
        action.sa_sigaction = handler as usize;
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: `action` is initialized and the old action is not requested.
        if unsafe { libc::sigaction(signal, &raw const action, std::ptr::null_mut()) } != 0 {
            return Err(errno());
        }
    }
    Ok(fds[0])
}

/// Waits for the next request on `read_end`, or `None` when the pipe is gone.
#[must_use]
pub fn next(read_end: i32) -> Option<Request> {
    loop {
        let mut poll = libc::pollfd {
            fd: read_end,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll` reads and writes the one valid `pollfd`.
        if unsafe { libc::poll(&raw mut poll, 1, -1) } < 0 && errno() != libc::EINTR {
            return None;
        }
        let mut byte = 0_u8;
        // SAFETY: `read` writes at most one byte into the valid local.
        let read = unsafe { libc::read(read_end, (&raw mut byte).cast(), 1) };
        if read == 1 {
            if let Some(request) = Request::from_signal(i32::from(byte)) {
                return Some(request);
            }
        } else if read == 0 {
            return None;
        }
    }
}

/// Installs the handlers and starts the thread that ends the machine on a request.
pub fn watch() {
    let read_end = match install() {
        Ok(read_end) => read_end,
        Err(errno) => {
            console::report(&format!("shutdown signals not installed: errno {errno}"));
            return;
        }
    };
    let spawned = thread::Builder::new()
        .name("power".to_owned())
        .spawn(move || {
            if let Some(request) = next(read_end) {
                end(request);
            }
        });
    if spawned.is_err() {
        console::report("shutdown signal watcher not started");
    }
}

/// Ends the machine for a request from inside the guest.
fn end(request: Request) -> ! {
    console::report(&format!("guest requested {request:?}"));
    // SAFETY: `kill(-1, …)` from init signals every process except init itself.
    unsafe { libc::kill(-1, libc::SIGTERM) };
    let deadline = Instant::now() + GRACE;
    while Instant::now() < deadline && !descendants::other_pids().is_empty() {
        pid1::reap_orphans();
        thread::sleep(Duration::from_millis(100));
    }
    descendants::sweep_strays();
    if matches!(request, Request::PowerOff | Request::Halt) {
        pid1::sync();
        console::line(soma_guest::POWER_OFF_LINE);
    }
    pid1::poweroff()
}

/// The commands every instance gets, whatever its image: one that asks init, and the four names
/// people and tools already use for it.
const COMMANDS: [(&str, &str); 5] = [
    ("miosa-power", include_str!("power/miosa-power.sh")),
    ("poweroff", include_str!("power/command.sh")),
    ("reboot", include_str!("power/command.sh")),
    ("halt", include_str!("power/command.sh")),
    ("shutdown", include_str!("power/command.sh")),
];

/// Where the commands are installed: early on every common `PATH`, and never a path an image's
/// package manager owns.
const COMMAND_DIRECTORY: &str = "/usr/local/sbin";

/// Installs the shut down and restart commands into the instance's root (SOMA#258).
///
/// The instance image may be any image, so the agent, which every instance runs, is what makes
/// the commands present. A command the image already ships is left alone, a root that cannot be
/// written is left as it is, and nothing here fails the boot.
pub fn install_commands() -> usize {
    if !std::path::Path::new("/bin/sh").exists() {
        return 0;
    }
    install_into(std::path::Path::new(COMMAND_DIRECTORY))
}

fn install_into(directory: &std::path::Path) -> usize {
    use std::os::unix::fs::PermissionsExt as _;

    if std::fs::create_dir_all(directory).is_err() {
        return 0;
    }
    let mut installed = 0;
    for (name, script) in COMMANDS {
        let path = directory.join(name);
        if path.symlink_metadata().is_ok() {
            continue;
        }
        if std::fs::write(&path, script).is_ok()
            && std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).is_ok()
        {
            installed += 1;
        }
    }
    installed
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_are_installed_once_and_an_image_s_own_is_kept() {
        let directory = std::env::temp_dir().join(format!("soma-power-{}", std::process::id()));
        let _ignored = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("directory");
        std::fs::write(directory.join("reboot"), b"image's own").expect("image command");
        assert_eq!(install_into(&directory), 4);
        assert_eq!(
            std::fs::read(directory.join("reboot")).expect("kept"),
            b"image's own"
        );
        assert!(
            std::fs::read_to_string(directory.join("miosa-power"))
                .expect("installed")
                .contains("kill \"-$signal\" 1")
        );
        assert_eq!(
            install_into(&directory),
            0,
            "a second boot installs nothing"
        );
        let _ignored = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn each_init_shutdown_signal_names_its_request() {
        assert_eq!(Request::from_signal(libc::SIGUSR2), Some(Request::PowerOff));
        assert_eq!(Request::from_signal(libc::SIGTERM), Some(Request::Restart));
        assert_eq!(Request::from_signal(libc::SIGUSR1), Some(Request::Halt));
        assert_eq!(
            Request::from_signal(libc::SIGINT),
            Some(Request::CtrlAltDel)
        );
        assert_eq!(Request::from_signal(libc::SIGHUP), None);
    }

    /// The test process is not init, so the request is read and never acted on.
    #[test]
    fn a_shutdown_signal_arrives_as_its_request() {
        let read_end = install().expect("handlers installed");
        // SAFETY: `raise` sends the signal to this process, whose handler only writes a byte.
        unsafe { libc::raise(libc::SIGUSR2) };
        assert_eq!(next(read_end), Some(Request::PowerOff));
    }
}
