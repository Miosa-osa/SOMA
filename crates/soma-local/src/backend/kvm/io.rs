//! The `soma-guest` host adapter over one sandbox machine's byte channel.
//!
//! Every read and write carries an absolute deadline to the channel. Committing repair is the
//! point at which the VMM verifies the guest erased the launch page and retires its slot, which
//! is why this adapter needs the machine and not only the channel.

use std::time::Instant;

use soma_guest::{ControlIo, HostControlIo};
use soma_kvm::x86_64::{ChannelError, ControlChannel, SandboxMachine};

/// The exact domain bytes at the start of every valid launch page.
pub(super) const PAGE_DOMAIN: &[u8] = b"SOMA-LAUNCH-PAGE";

pub(super) struct HostIo<'a> {
    channel: ControlChannel,
    sandbox: &'a SandboxMachine,
}

impl<'a> HostIo<'a> {
    pub(super) fn new(sandbox: &'a SandboxMachine) -> Self {
        Self {
            channel: sandbox.control(),
            sandbox,
        }
    }
}

impl ControlIo for HostIo<'_> {
    type Error = ChannelError;

    fn read_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), ChannelError> {
        self.channel.read_exact(bytes, deadline)
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), ChannelError> {
        self.channel.write_all(bytes, deadline)
    }

    fn poison(&mut self) {
        self.channel.poison();
    }
}

impl HostControlIo for HostIo<'_> {
    /// Retires the launch page slot, which is what makes repair irreversible.
    ///
    /// A failure here is reported as a closed channel: the session cannot continue against a
    /// machine whose launch material may still be mapped.
    fn commit_repair(&mut self, _deadline: Instant) -> Result<(), ChannelError> {
        self.sandbox
            .retire_launch_page()
            .map_err(|_| ChannelError::Closed)
    }
}
