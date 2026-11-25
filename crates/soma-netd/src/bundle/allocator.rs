//! The addresses and conntrack zones every bundle draws from.

use std::collections::BTreeSet;

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

impl Allocator {
    /// Takes out of circulation the lease indices and zones a previous process left assigned.
    ///
    /// A starting broker counts from zero, but a bundle the ledger still names keeps its
    /// addresses, routes and zone in the kernel. Handing those out again made every new bundle
    /// fail with `EEXIST` on its route, and the failed index went straight back to the free list,
    /// so no claim could succeed again (SOMA#238). Run before any bundle is built.
    pub(super) fn reserve(&mut self, leases: &BTreeSet<u32>, zones: &BTreeSet<u16>) {
        if let Some(&highest) = leases.last() {
            let mut taken = Vec::new();
            while let Ok(pair) = self.ipam.allocate() {
                let index = pair.guest.index();
                taken.push(index);
                if index >= highest {
                    break;
                }
            }
            for index in taken.into_iter().filter(|index| !leases.contains(index)) {
                self.ipam.recycle(index);
            }
        }
        if let Some(&highest) = zones.last() {
            while self.next_zone <= highest {
                if !zones.contains(&self.next_zone) {
                    self.free_zones.push(self.next_zone);
                }
                self.next_zone += 1;
            }
        }
    }
}

impl super::Broker {
    /// Reserves what every bundle the ledger still names as assigned in this generation holds,
    /// and answers how many it reserved.
    ///
    /// # Errors
    ///
    /// Returns the ledger's read failure; nothing is reserved then.
    pub fn reserve_assigned(&self) -> Result<usize, Error> {
        let mut leases = BTreeSet::new();
        let mut zones = BTreeSet::new();
        for entry in self.ledger.entries()? {
            if !entry.released && entry.record.generation == self.generation {
                leases.insert(entry.record.lease_index);
                zones.insert(entry.record.zone.get());
            }
        }
        self.allocator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reserve(&leases, &zones);
        Ok(leases.len())
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::{CleanupGeneration, SubnetPlan};

    fn allocator() -> Allocator {
        let plan = |third| SubnetPlan::new(Ipv4Addr::new(10, third, 0, 0), 16).expect("plan");
        let generation = CleanupGeneration::new(2).expect("generation");
        Allocator::new(Ipam::new(plan(200), plan(201), generation, 64).expect("ipam"))
    }

    #[test]
    fn a_restarted_broker_never_hands_out_what_an_assigned_bundle_still_holds() {
        let mut allocator = allocator();
        let held_leases = BTreeSet::from([0, 3, 7]);
        let held_zones = BTreeSet::from([1, 4]);
        allocator.reserve(&held_leases, &held_zones);
        let mut leases = BTreeSet::new();
        let mut zones = BTreeSet::new();
        for _ in 0..20 {
            let (pair, zone) = allocator.take().expect("take");
            assert!(
                leases.insert(pair.guest.index()),
                "an index is handed out once"
            );
            zones.insert(zone.get());
        }
        assert!(leases.is_disjoint(&held_leases), "{leases:?}");
        assert!(zones.is_disjoint(&held_zones), "{zones:?}");
        assert!(
            leases.contains(&1) && leases.contains(&2),
            "free indices are still used"
        );
    }

    #[test]
    fn nothing_held_reserves_nothing() {
        let mut allocator = allocator();
        allocator.reserve(&BTreeSet::new(), &BTreeSet::new());
        let (pair, zone) = allocator.take().expect("take");
        assert_eq!((pair.guest.index(), zone.get()), (0, 1));
    }
}
