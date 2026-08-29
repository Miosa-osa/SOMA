//! The `soma-guest` host adapter over the sandbox machine's byte channel.
//!
//! Every read and write forwards its absolute deadline to the channel; the repair commit is
//! the point at which the VMM verifies the launch page was erased and retires its slot.

use std::time::Instant;

use soma_guest::{ControlIo, HostControlIo};
use soma_kvm::x86_64::{ChannelError, ControlChannel, SandboxMachine};

pub struct HostIo<'a> {
    channel: ControlChannel,
    sandbox: &'a SandboxMachine,
}

impl<'a> HostIo<'a> {
    pub fn new(sandbox: &'a SandboxMachine) -> Self {
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
    fn commit_repair(&mut self, _deadline: Instant) -> Result<(), ChannelError> {
        self.sandbox.retire_launch_page().map_err(|error| {
            eprintln!("launch page retirement failed: {error}");
            ChannelError::Closed
        })
    }
}
