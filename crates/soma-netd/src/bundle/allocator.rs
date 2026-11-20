//! The addresses and conntrack zones every bundle draws from.

use crate::{ConntrackZone, Error, Ipam, LeasePair};

/// The lease and conntrack-zone counters, behind one lock so concurrent preparation cannot
/// hand two bundles the same addresses.
#[derive(Debug)]
pub(super) struct Allocator {
    ipam: Ipam,
    next_zone: u16,
    free_zones: Vec<u16>,
}

impl Allocator {
    /// An allocator over `ipam` whose first conntrack zone is 1.
    pub(super) fn new(ipam: Ipam) -> Self {
        Self {
            ipam,
            next_zone: 1,
            free_zones: Vec::new(),
        }
    }

    /// Takes one lease pair and one conntrack zone, or neither.
    pub(super) fn take(&mut self) -> Result<(LeasePair, ConntrackZone), Error> {
        let leases = self.ipam.allocate()?;
        let zone = if let Some(zone) = self.free_zones.pop() {
            zone
        } else if let Some(next) = self.next_zone.checked_add(1) {
            std::mem::replace(&mut self.next_zone, next)
        } else {
            self.ipam.recycle(leases.guest.index());
            return Err(Error::PoolExhausted);
        };
        Ok((leases, ConntrackZone::new(zone)?))
    }

    /// Makes a verified-released lease index and conntrack zone available again.
    pub(super) fn recycle(&mut self, lease_index: u32, zone: ConntrackZone) {
        self.ipam.recycle(lease_index);
        if zone.get() < self.next_zone && !self.free_zones.contains(&zone.get()) {
            self.free_zones.push(zone.get());
        }
    }
}
