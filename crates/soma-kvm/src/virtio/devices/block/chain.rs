//! Serve one virtio-blk descriptor chain under the device's limits.

use super::{
    BlockDevice, CHAIN_LIMITS, ChainHandler, ChainLimits, DescriptorChain, DeviceFault,
    GuestMemory, RequestError, VIRTIO_BLK_S_IOERR, parse_request, write_writable,
};

impl ChainHandler for BlockDevice {
    fn chain_limits(&self, _queue: u16) -> ChainLimits {
        CHAIN_LIMITS
    }

    fn handle_chain<M: GuestMemory + ?Sized>(
        &mut self,
        _queue: u16,
        chain: &DescriptorChain,
        mem: &M,
    ) -> Result<u32, DeviceFault> {
        let (status, data_len, status_skip, error) = match parse_request(mem, chain, self.limits())
        {
            Ok(request) => {
                let (status, data_len) = self.execute(request, chain, mem)?;
                (status, data_len, request.status_skip, None)
            }
            Err(RequestError::NoStatus) => {
                self.count(VIRTIO_BLK_S_IOERR, Some(RequestError::NoStatus));
                return Ok(0);
            }
            Err(error) => (
                error.status(),
                0,
                chain.writable_len().saturating_sub(1),
                Some(error),
            ),
        };
        self.count(status, error);
        if write_writable(mem, chain, status_skip, &[status])? != 1 {
            return Err(DeviceFault::Protocol);
        }
        Ok(data_len.saturating_add(1))
    }
}
