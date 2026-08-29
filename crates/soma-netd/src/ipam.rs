//! Deterministic lease allocation and identity derivation for one bundle.
//!
//! Every lease is one IPv4 `/30`: the first usable address belongs to the broker gateway inside
//! the sandbox namespace and the second to the guest.
//! A parallel transit `/30` connects the sandbox namespace to the host through the veth pair.
//! Indices are never reused inside one allocator generation, so a released lease cannot be
//! handed to a later Instance until the broker deliberately starts a new generation.

use std::net::Ipv4Addr;

use sha2::{Digest, Sha256};

use crate::{BundleId, Cidr, CleanupGeneration, Error};

const LEASE_PREFIX: u8 = 30;
const MIN_PLAN_PREFIX: u8 = 8;
const MAX_PLAN_PREFIX: u8 = 28;

/// One IPv4 plan that is carved into consecutive `/30` leases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubnetPlan {
    cidr: Cidr,
}

impl SubnetPlan {
    /// Validates one plan between `/8` and `/28`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] for a prefix outside that range.
    pub fn new(base: Ipv4Addr, prefix: u8) -> Result<Self, Error> {
        if !(MIN_PLAN_PREFIX..=MAX_PLAN_PREFIX).contains(&prefix) {
            return Err(Error::InvalidProfile("plan prefix length"));
        }
        Ok(Self {
            cidr: Cidr::v4(base, prefix)?,
        })
    }

    /// Returns the plan prefix.
    #[must_use]
    pub const fn cidr(&self) -> Cidr {
        self.cidr
    }

    /// Returns how many `/30` leases the plan holds.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        let Cidr::V4(_, prefix) = self.cidr else {
            return 0;
        };
        1 << (LEASE_PREFIX - prefix)
    }

    /// Returns the lease at one index.
    #[must_use]
    pub fn lease(&self, index: u32) -> Option<Lease> {
        let Cidr::V4(base, _) = self.cidr else {
            return None;
        };
        if index >= self.capacity() {
            return None;
        }
        let network = base.to_bits().checked_add(index.checked_mul(4)?)?;
        Some(Lease {
            network: Ipv4Addr::from_bits(network),
            index,
        })
    }
}

/// One allocated `/30`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lease {
    network: Ipv4Addr,
    index: u32,
}

impl Lease {
    /// Returns the allocator index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Returns the `/30` prefix.
    #[must_use]
    pub const fn cidr(self) -> Cidr {
        Cidr::V4(self.network, LEASE_PREFIX)
    }

    /// Returns the host-side address, the first usable address.
    #[must_use]
    pub fn host(self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.network.to_bits() + 1)
    }

    /// Returns the guest-side address, the second usable address.
    #[must_use]
    pub fn guest(self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.network.to_bits() + 2)
    }

    /// Returns the prefix length delivered to the guest.
    #[must_use]
    pub const fn prefix_length(self) -> u8 {
        LEASE_PREFIX
    }
}

/// The lease and transit pair reserved for one bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeasePair {
    /// The guest-facing lease; `host()` is the gateway and `guest()` the Instance.
    pub guest: Lease,
    /// The veth transit lease; `host()` is the host end and `guest()` the sandbox end.
    pub transit: Lease,
}

/// A bounded, generation-scoped allocator over one lease plan and one transit plan.
#[derive(Debug)]
pub struct Ipam {
    leases: SubnetPlan,
    transit: SubnetPlan,
    generation: CleanupGeneration,
    next: u32,
    limit: u32,
}

impl Ipam {
    /// Creates an allocator bounded by `limit` leases or the smaller plan capacity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] when the limit is zero.
    pub fn new(
        leases: SubnetPlan,
        transit: SubnetPlan,
        generation: CleanupGeneration,
        limit: u32,
    ) -> Result<Self, Error> {
        if limit == 0 {
            return Err(Error::InvalidProfile("lease limit"));
        }
        let limit = limit.min(leases.capacity()).min(transit.capacity());
        Ok(Self {
            leases,
            transit,
            generation,
            next: 0,
            limit,
        })
    }

    /// Returns the allocator generation.
    #[must_use]
    pub const fn generation(&self) -> CleanupGeneration {
        self.generation
    }

    /// Allocates the next lease pair; indices never repeat inside one generation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PoolExhausted`] when the bounded pool is used up.
    pub fn allocate(&mut self) -> Result<LeasePair, Error> {
        if self.next >= self.limit {
            return Err(Error::PoolExhausted);
        }
        let index = self.next;
        self.next += 1;
        let guest = self.leases.lease(index).ok_or(Error::PoolExhausted)?;
        let transit = self.transit.lease(index).ok_or(Error::PoolExhausted)?;
        Ok(LeasePair { guest, transit })
    }

    /// Starts a new generation, allowing released indices to be reused.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidId`] when the generation counter would wrap.
    pub fn next_generation(&mut self) -> Result<CleanupGeneration, Error> {
        let next = self
            .generation
            .get()
            .checked_add(1)
            .ok_or(Error::InvalidId("generation"))?;
        self.generation = CleanupGeneration::new(next)?;
        self.next = 0;
        Ok(self.generation)
    }
}

/// Derives the locally administered unicast MAC pair for one bundle.
///
/// The first byte is `0x02` for the guest and `0x06` for the broker TAP end; the remaining
/// five bytes are the SHA-256 prefix of a domain-separated bundle identity.
#[must_use]
pub fn derive_macs(bundle: BundleId) -> MacPair {
    let mut hash = Sha256::new();
    hash.update(b"soma-netd-mac-v1\0");
    hash.update(bundle.as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    let mut guest = [0x02, 0, 0, 0, 0, 0];
    let mut tap = [0x06, 0, 0, 0, 0, 0];
    guest[1..].copy_from_slice(&digest[..5]);
    tap[1..].copy_from_slice(&digest[..5]);
    MacPair { guest, tap }
}

/// The guest and TAP MAC addresses of one bundle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacPair {
    /// The MAC delivered to the guest.
    pub guest: [u8; 6],
    /// The MAC installed on the broker-side TAP.
    pub tap: [u8; 6],
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn plan(third: u8) -> SubnetPlan {
        SubnetPlan::new(Ipv4Addr::new(10, third, 0, 0), 16).expect("plan")
    }

    #[test]
    fn leases_are_consecutive_slash_30s_with_gateway_then_guest() {
        let lease = plan(200).lease(3).expect("in range");
        assert_eq!(lease.cidr().to_string(), "10.200.0.12/30");
        assert_eq!(lease.host(), Ipv4Addr::new(10, 200, 0, 13));
        assert_eq!(lease.guest(), Ipv4Addr::new(10, 200, 0, 14));
        assert_eq!(lease.prefix_length(), 30);
        assert_eq!(plan(200).capacity(), 16_384);
        assert!(plan(200).lease(16_384).is_none());
        assert_eq!(
            SubnetPlan::new(Ipv4Addr::new(10, 0, 0, 0), 30).expect_err("too small"),
            Error::InvalidProfile("plan prefix length")
        );
    }

    #[test]
    fn allocator_is_bounded_and_never_reuses_inside_a_generation() {
        let generation = CleanupGeneration::new(1).expect("nonzero");
        let mut ipam = Ipam::new(plan(200), plan(201), generation, 2).expect("ipam");
        let first = ipam.allocate().expect("first");
        let second = ipam.allocate().expect("second");
        assert_eq!(first.guest.index(), 0);
        assert_eq!(second.guest.index(), 1);
        assert_eq!(second.transit.host(), Ipv4Addr::new(10, 201, 0, 5));
        assert_eq!(
            ipam.allocate().expect_err("exhausted"),
            Error::PoolExhausted
        );
        assert_eq!(ipam.next_generation().expect("bump").get(), 2);
        assert_eq!(
            ipam.allocate()
                .expect("reuse in new generation")
                .guest
                .index(),
            0
        );
        assert_eq!(
            Ipam::new(plan(200), plan(201), generation, 0).expect_err("zero"),
            Error::InvalidProfile("lease limit")
        );
    }

    #[test]
    fn macs_are_locally_administered_unicast_and_unique_per_bundle() {
        let mut seen = BTreeSet::new();
        for index in 1..=512_u32 {
            let mut bytes = [0; 16];
            bytes[..4].copy_from_slice(&index.to_be_bytes());
            let macs = derive_macs(BundleId::new(bytes).expect("nonzero"));
            assert_eq!(macs.guest[0] & 0b11, 0b10);
            assert_eq!(macs.tap[0] & 0b11, 0b10);
            assert_ne!(macs.guest, macs.tap);
            assert!(seen.insert(macs.guest), "guest MAC collision at {index}");
            assert!(seen.insert(macs.tap), "tap MAC collision at {index}");
        }
        let bundle = BundleId::new([7; 16]).expect("nonzero");
        assert_eq!(derive_macs(bundle), derive_macs(bundle));
    }
}
