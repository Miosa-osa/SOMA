//! The parent side of the probe's control socket and the per-jail driver used by the tests.

#![allow(unsafe_code)]

use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    time::{Duration, Instant},
};

use soma_jail::{ExitReason, JailEvidence, JailHandle, ProbeCommand, ProbeReport};

use super::harness::{Live, deadline};

const RECV_TIMEOUT: Duration = Duration::from_secs(5);

fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// The parent end of the probe's `SOCK_SEQPACKET` control socket.
pub(crate) struct Control(pub(crate) OwnedFd);

impl Control {
    /// One packet, or `None` when the probe closed the socket or stayed silent for `timeout`.
    pub(crate) fn recv(&self, timeout: Duration) -> Option<String> {
        let mut poll = libc::pollfd {
            fd: self.0.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = libc::c_int::try_from(timeout.as_millis()).unwrap_or(libc::c_int::MAX);
        // SAFETY: `poll` receives one valid `pollfd` and its count.
        if unsafe { libc::poll(&raw mut poll, 1, millis) } <= 0 {
            return None;
        }
        let mut buffer = [0u8; 1024];
        // SAFETY: the buffer and length describe valid writable storage.
        let received = unsafe {
            libc::recv(
                self.0.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                0,
            )
        };
        let count = usize::try_from(received).ok().filter(|count| *count > 0)?;
        Some(String::from_utf8_lossy(&buffer[..count]).into_owned())
    }

    pub(crate) fn send(&self, command: ProbeCommand) {
        self.send_text(&command.encode());
    }

    /// Sends one already encoded packet, which is how the `soma-vmm` worker is driven.
    pub(crate) fn send_text(&self, text: &str) {
        // SAFETY: the buffer and length describe a live string slice.
        let sent = unsafe { libc::send(self.0.as_raw_fd(), text.as_ptr().cast(), text.len(), 0) };
        assert_eq!(
            usize::try_from(sent).ok(),
            Some(text.len()),
            "send {text:?}: errno {}",
            errno()
        );
    }
}

/// One launched probe with its control socket.
pub(crate) struct Jail {
    pub handle: JailHandle,
    pub control: Control,
}

impl Jail {
    /// How the child ended, for failure messages; `still running` if it has not.
    fn describe_exit(&mut self) -> String {
        match self
            .handle
            .wait(Instant::now() + Duration::from_millis(200))
        {
            Ok(exit) => exit.to_string(),
            Err(error) => format!("still running ({error})"),
        }
    }

    /// The probe's first packet, decoded strictly.
    pub(crate) fn report(&mut self) -> ProbeReport {
        let Some(text) = self.control.recv(RECV_TIMEOUT) else {
            panic!("no probe report; child {}", self.describe_exit());
        };
        ProbeReport::decode(&text).unwrap_or_else(|error| panic!("{error}: {text:?}"))
    }

    /// Sends `command` and expects a reply starting with `prefix`.
    pub(crate) fn expect(&mut self, command: ProbeCommand, prefix: &str) -> String {
        self.expect_text(&command.encode(), prefix)
    }

    /// Sends one packet and expects a reply starting with `prefix`.
    pub(crate) fn expect_text(&mut self, packet: &str, prefix: &str) -> String {
        self.control.send_text(packet);
        let Some(reply) = self.control.recv(RECV_TIMEOUT) else {
            panic!("no reply to {packet}; child {}", self.describe_exit());
        };
        assert!(reply.starts_with(prefix), "reply to {packet}: {reply:?}");
        reply
    }

    /// Sends `command` and expects the probe to die without replying.
    pub(crate) fn expect_silence(&self, command: ProbeCommand) {
        self.expect_silence_text(&command.encode());
    }

    /// Sends one packet and expects the child to die without replying.
    pub(crate) fn expect_silence_text(&self, packet: &str) {
        self.control.send_text(packet);
        let reply = self.control.recv(RECV_TIMEOUT);
        assert_eq!(reply, None, "child survived {packet}");
    }

    /// Waits for `expected`, reconciles, and proves nothing is left behind.
    pub(crate) fn finish(mut self, live: &Live, expected: ExitReason) -> JailEvidence {
        let exit = self.handle.wait(deadline(10)).expect("child exit");
        assert_eq!(exit, expected, "{exit}");
        let record = self.handle.ledger().record();
        let (disposition, evidence) = self.handle.reconcile(deadline(5));
        assert!(disposition.is_released(), "{disposition}");
        assert_eq!(evidence.exit, Some(expected));
        live.assert_zero_residual(&record);
        evidence
    }

    /// Asks the probe to exit with `code` and proves a clean release.
    pub(crate) fn exit(self, live: &Live, code: i32) -> JailEvidence {
        self.control.send(ProbeCommand::Exit(code));
        self.finish(live, ExitReason::Exited(code))
    }

    /// Sends a command the filter must kill and proves the recorded `SIGSYS` death.
    pub(crate) fn expect_kill(self, live: &Live, command: ProbeCommand) -> JailEvidence {
        self.expect_kill_text(live, &command.encode())
    }

    /// Sends one packet the filter must kill and proves the recorded `SIGSYS` death.
    pub(crate) fn expect_kill_text(mut self, live: &Live, packet: &str) -> JailEvidence {
        self.expect_silence_text(packet);
        let exit = self.handle.wait(deadline(10)).expect("child exit");
        assert!(exit.is_seccomp_kill(), "{packet}: {exit}");
        self.finish(live, exit)
    }
}
