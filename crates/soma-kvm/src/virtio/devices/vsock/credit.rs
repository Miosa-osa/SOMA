//! Stream credit accounting for one connection.
//!
//! The protocol counters (`tx_cnt`, `rx_cnt`, `fwd_cnt`) are 32-bit values
//! that advance modulo 2^32 by definition; every derived quantity (free
//! credit, bytes in flight) is computed with checked arithmetic and an
//! impossibility check, so a hostile `fwd_cnt` or `buf_alloc` can never
//! authorize more bytes than either side actually buffers.

use std::fmt;

/// Receive buffer the host advertises to the guest.
pub const HOST_BUF_ALLOC: u32 = 1 << 16;

/// Why a credit operation was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreditError {
    /// The peer's `fwd_cnt` acknowledges more than we ever sent, or its
    /// `buf_alloc` is smaller than the bytes it has not yet consumed.
    ImpossibleAccounting { buf_alloc: u32, fwd_cnt: u32 },
    /// The guest sent more than the host advertised.
    Exceeded { len: u32 },
}

impl fmt::Display for CreditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "vsock credit rejected: {self:?}")
    }
}

impl std::error::Error for CreditError {}

/// Both directions of credit for one connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Credit {
    peer_buf_alloc: u32,
    peer_fwd_cnt: u32,
    tx_cnt: u32,
    rx_cnt: u32,
    fwd_cnt: u32,
}

impl Credit {
    /// Fresh accounting from the peer's first advertised window.
    ///
    /// # Errors
    /// Rejects an initial `fwd_cnt` other than zero relative to nothing sent.
    pub fn new(peer_buf_alloc: u32, peer_fwd_cnt: u32) -> Result<Self, CreditError> {
        let mut credit = Self {
            peer_buf_alloc: 0,
            peer_fwd_cnt: 0,
            tx_cnt: 0,
            rx_cnt: 0,
            fwd_cnt: 0,
        };
        credit.update_peer(peer_buf_alloc, peer_fwd_cnt)?;
        Ok(credit)
    }

    /// Applies the peer's `buf_alloc` and `fwd_cnt` from any received packet.
    ///
    /// # Errors
    /// Rejects accounting that cannot be true given what was sent.
    pub fn update_peer(&mut self, buf_alloc: u32, fwd_cnt: u32) -> Result<(), CreditError> {
        let unconsumed = self.tx_cnt.wrapping_sub(fwd_cnt);
        if unconsumed > buf_alloc {
            return Err(CreditError::ImpossibleAccounting { buf_alloc, fwd_cnt });
        }
        self.peer_buf_alloc = buf_alloc;
        self.peer_fwd_cnt = fwd_cnt;
        Ok(())
    }

    /// Bytes the host may send now.
    #[must_use]
    pub fn peer_free(&self) -> u32 {
        let unconsumed = self.tx_cnt.wrapping_sub(self.peer_fwd_cnt);
        self.peer_buf_alloc.saturating_sub(unconsumed)
    }

    /// Accounts `len` bytes sent to the guest; the caller bounded it by [`Self::peer_free`].
    pub const fn sent(&mut self, len: u32) {
        self.tx_cnt = self.tx_cnt.wrapping_add(len);
    }

    /// Accepts `len` bytes from the guest if they fit the advertised window.
    ///
    /// # Errors
    /// Rejects bytes beyond [`HOST_BUF_ALLOC`] minus what the host has not consumed.
    pub fn accept_rx(&mut self, len: u32) -> Result<(), CreditError> {
        let in_flight = self.rx_cnt.wrapping_sub(self.fwd_cnt);
        match in_flight.checked_add(len) {
            Some(total) if total <= HOST_BUF_ALLOC => {
                self.rx_cnt = self.rx_cnt.wrapping_add(len);
                Ok(())
            }
            _ => Err(CreditError::Exceeded { len }),
        }
    }

    /// Accounts `len` bytes the host reader consumed.
    pub const fn consumed(&mut self, len: u32) {
        self.fwd_cnt = self.fwd_cnt.wrapping_add(len);
    }

    /// The `buf_alloc` and `fwd_cnt` to place in every outgoing header.
    #[must_use]
    pub const fn local_fields(&self) -> (u32, u32) {
        (HOST_BUF_ALLOC, self.fwd_cnt)
    }

    /// Bytes received from the guest and not yet consumed by the host.
    #[must_use]
    pub const fn in_flight(&self) -> u32 {
        self.rx_cnt.wrapping_sub(self.fwd_cnt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_credit_is_derived_with_impossibility_checks() {
        let mut credit = Credit::new(1024, 0).expect("fresh");
        assert_eq!(credit.peer_free(), 1024);
        credit.sent(1000);
        assert_eq!(credit.peer_free(), 24);
        assert_eq!(
            credit.update_peer(1024, 1001),
            Err(CreditError::ImpossibleAccounting {
                buf_alloc: 1024,
                fwd_cnt: 1001
            })
        );
        assert_eq!(
            credit.update_peer(512, 100),
            Err(CreditError::ImpossibleAccounting {
                buf_alloc: 512,
                fwd_cnt: 100
            })
        );
        credit.update_peer(1024, 1000).expect("consumed all");
        assert_eq!(credit.peer_free(), 1024);
        assert!(Credit::new(0, 5).is_err());
        let mut wrap = Credit::new(16, 0).expect("fresh");
        wrap.tx_cnt = u32::MAX - 3;
        wrap.peer_fwd_cnt = u32::MAX - 3;
        wrap.sent(8);
        assert_eq!(
            wrap.peer_free(),
            8,
            "wrapping counters still derive correctly"
        );
    }

    #[test]
    fn host_window_bounds_guest_bytes_until_consumed() {
        let mut credit = Credit::new(4096, 0).expect("fresh");
        credit.accept_rx(HOST_BUF_ALLOC).expect("fills window");
        assert_eq!(credit.accept_rx(1), Err(CreditError::Exceeded { len: 1 }));
        assert_eq!(
            credit.accept_rx(u32::MAX),
            Err(CreditError::Exceeded { len: u32::MAX })
        );
        credit.consumed(100);
        credit.accept_rx(100).expect("space reopened");
        assert_eq!(credit.in_flight(), HOST_BUF_ALLOC);
        assert_eq!(credit.local_fields(), (HOST_BUF_ALLOC, 100));
    }
}
