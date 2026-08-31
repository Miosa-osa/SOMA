//! Allocating a real pseudo-terminal pair and running a shell as its session leader.
//!
//! The pair is opened through the kernel's own interface rather than through the C library's
//! conveniences. `openpty` and `login_tty` live in libutil, `grantpt` has historically forked a
//! helper program, and `ptsname` answers into a static buffer; the agent is a statically linked
//! binary with no name-service plugins and nothing to fork a helper from, so it unlocks the pair
//! and asks for its number with the two `ioctl` calls those wrappers make anyway.
//!
//! The child has to become a session leader and take the slave as its controlling terminal, or
//! it is a process holding a terminal descriptor rather than a process sitting at a terminal:
//! job control would not work, a signal typed at the terminal would reach nothing, and a shell
//! would notice and complain. Both calls happen between the fork and the `execve`, where the
//! standard library has already made the slave the child's standard input, output, and error.

#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Child, Command, Stdio};

use soma_guest::PtySize;

use crate::environment::{ENVIRONMENT, WORKING_DIRECTORY};

/// The multiplexer every Linux pseudo-terminal is allocated from.
const MULTIPLEXER: &str = "/dev/ptmx";
/// The shell a terminal session runs.
///
/// The path is fixed rather than taken from the caller because a terminal is not a command: an
/// interactive session that could name its own program would be a second execution path with
/// none of the bounds the command path applies to argv, environment, and output.
const SHELL: &str = "/bin/sh";
/// What a program in this terminal is told the terminal is.
///
/// The agent's base environment says `dumb`, which is right for a command whose output is
/// captured and wrong for a session a caller draws into, so this one name is overridden.
const TERM: &str = "xterm-256color";

/// One allocated pseudo-terminal pair.
pub(super) struct Pair {
    /// The end the agent reads from and writes to.
    pub(super) master: File,
    /// The end the shell has as its standard input, output, and error.
    pub(super) slave: File,
}

/// Allocates one pseudo-terminal pair at the given dimensions.
pub(super) fn open(size: PtySize) -> io::Result<Pair> {
    let master = OpenOptions::new()
        .read(true)
        .write(true)
        // The agent is PID 1 and has no controlling terminal to lose, but asking for none is
        // what makes that true by construction rather than by circumstance.
        .custom_flags(libc::O_NOCTTY)
        .open(MULTIPLEXER)?;
    unlock(&master)?;
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(format!("/dev/pts/{}", number(&master)?))?;
    set_size(&master, size)?;
    Ok(Pair { master, slave })
}

/// Tells the terminal its dimensions, which is what a program reads back and what a resize
/// delivers `SIGWINCH` for.
pub(super) fn set_size(terminal: &File, size: PtySize) -> io::Result<()> {
    let window = libc::winsize {
        ws_row: size.rows(),
        ws_col: size.columns(),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `TIOCSWINSZ` reads one `winsize` from the pointer, and the local outlives the call.
    let result = unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCSWINSZ, &raw const window) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Reads back the dimensions the terminal is currently at.
///
/// Only the tests ask this: the protocol never reports a size the caller did not just set, so
/// the shipped agent has no reason to read one back.
#[cfg(test)]
pub(super) fn size_of(terminal: &File) -> io::Result<(u16, u16)> {
    let mut window = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `TIOCGWINSZ` writes one `winsize` through the pointer, and the local outlives it.
    let result = unsafe { libc::ioctl(terminal.as_raw_fd(), libc::TIOCGWINSZ, &raw mut window) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((window.ws_col, window.ws_row))
}

/// Spawns the shell with the slave as its standard input, output, and error.
pub(super) fn spawn(slave: &File) -> io::Result<Child> {
    let mut command = Command::new(SHELL);
    command
        .arg("-i")
        .env_clear()
        .envs(ENVIRONMENT.iter().copied())
        .env("TERM", TERM)
        .current_dir(WORKING_DIRECTORY)
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    // SAFETY: the closure runs between `fork` and `execve` in a single-threaded child, and calls
    // only `setsid` and `ioctl`, both of which are safe to use there.
    unsafe {
        command.pre_exec(become_session_leader);
    }
    command.spawn()
}

/// Makes the child its own session leader with the slave as its controlling terminal.
///
/// The standard library has already made the slave descriptor zero by this point, which is why
/// the `ioctl` names it rather than reaching for the file the parent still holds.
fn become_session_leader() -> io::Result<()> {
    // SAFETY: `setsid` takes no arguments and only changes the calling process's session.
    if unsafe { libc::setsid() } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `TIOCSCTTY` takes an integer argument by value and reads no memory.
    if unsafe { libc::ioctl(0, libc::TIOCSCTTY, 0 as libc::c_int) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Clears the lock every freshly allocated pair starts with.
fn unlock(master: &File) -> io::Result<()> {
    let unlocked: libc::c_int = 0;
    // SAFETY: `TIOCSPTLCK` reads one `int` from the pointer, and the local outlives the call.
    let result = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSPTLCK, &raw const unlocked) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Asks which slave in `/dev/pts` belongs to this master.
fn number(master: &File) -> io::Result<libc::c_uint> {
    let mut number: libc::c_uint = 0;
    // SAFETY: `TIOCGPTN` writes one `unsigned int` through the pointer, which outlives the call.
    let result = unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCGPTN, &raw mut number) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(number)
}
