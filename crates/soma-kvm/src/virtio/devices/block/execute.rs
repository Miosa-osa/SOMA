//! Execution of one validated [`BlockOp`] against the backend.

use super::request::{
    BlockOp, BlockRequest, REQUEST_HEADER_LEN, RequestError, VIRTIO_BLK_S_IOERR, VIRTIO_BLK_S_OK,
};
use super::{BLOCK_SERIAL_LEN, BlockDevice};
use crate::virtio::devices::segments::{read_readable, write_writable};
use crate::virtio::devices::service::DeviceFault;
use crate::virtio::guest_memory::GuestMemory;
use crate::virtio::queue::chain::DescriptorChain;

impl BlockDevice {
    /// Runs a validated operation; returns the status and guest data bytes written.
    pub(super) fn execute<M: GuestMemory + ?Sized>(
        &mut self,
        request: BlockRequest,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<(u8, u32), DeviceFault> {
        let outcome = match request.op {
            BlockOp::Read { offset, len } => {
                let size = usize::try_from(len).map_err(|_| DeviceFault::Protocol)?;
                let mut buf = vec![0u8; size];
                match self.backend.read_at(offset, &mut buf) {
                    Ok(read) if read == size => {
                        if write_writable(mem, chain, 0, &buf)? != size {
                            return Err(DeviceFault::Protocol);
                        }
                        (VIRTIO_BLK_S_OK, len)
                    }
                    _ => (VIRTIO_BLK_S_IOERR, 0),
                }
            }
            BlockOp::Write { offset, len } => {
                let size = usize::try_from(len).map_err(|_| DeviceFault::Protocol)?;
                let mut buf = vec![0u8; size];
                if read_readable(mem, chain, REQUEST_HEADER_LEN, &mut buf)? != size {
                    return Err(DeviceFault::Protocol);
                }
                match self.backend.write_at(offset, &buf) {
                    Ok(written) if written == size => (VIRTIO_BLK_S_OK, 0),
                    _ => (VIRTIO_BLK_S_IOERR, 0),
                }
            }
            BlockOp::Flush => match self.backend.flush() {
                Ok(()) => (VIRTIO_BLK_S_OK, 0),
                Err(_) => (VIRTIO_BLK_S_IOERR, 0),
            },
            BlockOp::GetId { len } => {
                let size = usize::try_from(len)
                    .ok()
                    .filter(|size| *size <= BLOCK_SERIAL_LEN)
                    .ok_or(DeviceFault::Protocol)?;
                if write_writable(mem, chain, 0, &self.serial[..size])? != size {
                    return Err(DeviceFault::Protocol);
                }
                (VIRTIO_BLK_S_OK, len)
            }
        };
        Ok(outcome)
    }

    pub(super) fn count(&mut self, status: u8, error: Option<RequestError>) {
        let slot = match (status, error) {
            (_, Some(RequestError::UnsupportedType { .. })) => &mut self.counters.unsupported,
            (_, Some(_)) => &mut self.counters.malformed,
            (VIRTIO_BLK_S_OK, None) => &mut self.counters.ok,
            _ => &mut self.counters.io_error,
        };
        *slot = slot.saturating_add(1);
    }
}
