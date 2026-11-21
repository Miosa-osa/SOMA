//! The control descriptor, the worker's only conversation with its supervisor.
//!
//! The launcher seals a `SOCK_SEQPACKET` socket into the manifest slot, so one packet in is
//! one request and one packet out is one reply, with no framing of our own to get wrong.
//! Both `sendto` and `recvmsg` stay admitted after the steady-state filter narrows, which is
//! why the worker keeps talking after it has sealed itself.

#![allow(unsafe_code)]

/// One end of the control socket, addressed by its manifest slot.
pub struct Channel {
    descriptor: libc::c_int,
}

impl Channel {
    /// Addresses the control descriptor, or `None` when the slot is not a descriptor number.
    ///
    /// The slot is not verified here: the attestation verifies the whole sealed table,
    /// including this slot's kind, before anything is served.
    pub fn open(slot: u32) -> Option<Self> {
        libc::c_int::try_from(slot)
            .ok()
            .map(|descriptor| Self { descriptor })
    }

    /// Sends one packet, dropping it if the supervisor has already gone; a worker that cannot
    /// report has nothing to do but reach its next exit.
    pub fn send(&self, text: &str) {
        // SAFETY: the pointer and length describe a live string slice.
        unsafe { libc::send(self.descriptor, text.as_ptr().cast(), text.len(), 0) };
    }

    /// Receives one packet and any descriptor capabilities that arrived with it.
    pub fn receive(
        &self,
        buffer: &mut [u8],
    ) -> Result<(usize, Vec<std::os::fd::OwnedFd>), soma_supervise::DescriptorHandoffError> {
        // SAFETY: the sealed descriptor table owns the connected control fd throughout service.
        let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(self.descriptor) };
        soma_supervise::receive_descriptor_packet(fd, buffer)
    }
}
