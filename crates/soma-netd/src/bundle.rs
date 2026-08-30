//! Sterile bundle preparation and atomic Instance assignment.
//!
//! `prepare` builds a namespace, a down TAP with the gateway address, a down veth pair whose
//! peer already sits in the namespace, the host zone and masquerade table, a fully denied
//! sandbox ruleset, and forwarding off.
//! `assign` records ownership in the ledger before it re-renders the ruleset for the admitted
//! intent, reserves ports, and produces the exact launch-page values.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use soma_guest::{ActivationChallenge, LaunchNetwork};

use crate::{
    AssignmentRecord, BundleId, BundleNames, CleanupGeneration, ConntrackZone, DnsPlan, Error,
    InstanceId, Ipam, Ledger, NetworkIntent, NetworkProfile, OperationId, PortReservation,
    RecordOutcome, SandboxRuleset, Step, derive_macs, ingress, namespace::NetNamespace, nft,
    release,
};

mod prepare;
mod types;

pub use types::{AssignFailure, Assigned, SterileBundle};

type AssignedParts = (
    AssignmentRecord,
    LaunchNetwork,
    Vec<PortReservation>,
    ActivationChallenge,
);

/// The broker state shared by every bundle operation.
#[derive(Debug)]
pub struct Broker {
    profile: NetworkProfile,
    ns_dir: PathBuf,
    ledger: Ledger,
    ipam: Ipam,
    next_zone: u16,
}

impl Broker {
    /// Opens the broker over one state directory after proving namespace privilege.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingPrivilege`] without `CAP_NET_ADMIN`, or a ledger or IPAM error.
    pub fn open(
        profile: NetworkProfile,
        state_dir: &Path,
        generation: CleanupGeneration,
        limit: u32,
    ) -> Result<Self, Error> {
        NetNamespace::probe_privilege()?;
        let ledger = Ledger::open(&state_dir.join("ledger"))?;
        let ipam = Ipam::new(
            profile.leases().clone(),
            profile.transit().clone(),
            generation,
            limit,
        )?;
        Ok(Self {
            profile,
            ns_dir: state_dir.join("ns"),
            ledger,
            ipam,
            next_zone: 1,
        })
    }

    /// Returns the served profile.
    #[must_use]
    pub const fn profile(&self) -> &NetworkProfile {
        &self.profile
    }

    /// Returns the ledger.
    #[must_use]
    pub const fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Returns the namespace pin directory.
    #[must_use]
    pub fn namespace_dir(&self) -> &Path {
        &self.ns_dir
    }

    /// Returns the current cleanup generation.
    #[must_use]
    pub const fn generation(&self) -> CleanupGeneration {
        self.ipam.generation()
    }

    /// Prepares one sterile bundle; any failure tears down what was built.
    ///
    /// # Errors
    ///
    /// Returns the first typed failure after rollback.
    pub fn prepare(&mut self, id: BundleId) -> Result<SterileBundle, Error> {
        let leases = self.ipam.allocate()?;
        let zone = ConntrackZone::new(self.next_zone)?;
        self.next_zone = self.next_zone.checked_add(1).ok_or(Error::PoolExhausted)?;
        let names = BundleNames::new(&id.short_hex());
        let short = id.short_hex();
        let namespace = NetNamespace::create(&self.ns_dir, &short)?;
        match prepare::build(self, &names, &namespace, leases, zone, id) {
            Ok(tap) => Ok(SterileBundle {
                id,
                generation: self.generation(),
                names,
                namespace,
                tap,
                leases,
                macs: derive_macs(id),
                zone,
            }),
            Err(error) => {
                drop(namespace);
                let _ =
                    release::teardown(&names, zone, &self.ns_dir.join(&short), None, Vec::new());
                Err(error)
            }
        }
    }

    /// Atomically assigns one sterile bundle to an Instance.
    ///
    /// # Errors
    ///
    /// `claim` is the guest vsock CID and the kernel-derived user identity of the control peer
    /// making the claim; the identity is recorded durably, so a release after a broker restart
    /// is still bound to the peer that claimed the bundle.
    ///
    /// # Errors
    ///
    /// Returns a ledger conflict or replay mismatch before any kernel change, or the first
    /// typed failure afterwards, together with the bundle so the caller can release it.
    pub fn assign(
        &self,
        bundle: SterileBundle,
        instance: InstanceId,
        operation: OperationId,
        intent: &NetworkIntent,
        claim: (u32, u32),
    ) -> Result<Assigned, AssignFailure> {
        match self.try_assign(&bundle, instance, operation, intent, claim) {
            Ok((record, launch, reservations, activation)) => Ok(Assigned {
                bundle,
                record,
                launch,
                reservations,
                activation: Some(activation),
                activated: None,
                active: false,
            }),
            Err(error) => Err(AssignFailure {
                bundle: Box::new(bundle),
                error,
            }),
        }
    }

    fn try_assign(
        &self,
        bundle: &SterileBundle,
        instance: InstanceId,
        operation: OperationId,
        intent: &NetworkIntent,
        claim: (u32, u32),
    ) -> Result<AssignedParts, Error> {
        let (vsock_cid, owner) = claim;
        let time_sample_nanos = now_nanos()?;
        let mut record = AssignmentRecord {
            bundle: bundle.id,
            generation: bundle.generation,
            instance,
            operation,
            owner,
            profile: self.profile.digest(),
            intent_digest: intent.digest(),
            guest_mac: bundle.macs.guest,
            lease_index: bundle.leases.guest.index(),
            transit_index: bundle.leases.transit.index(),
            zone: bundle.zone,
            vsock_cid,
            time_sample_nanos,
            intent: intent.clone(),
        };
        match self.ledger.record_assignment(&record)? {
            RecordOutcome::Recorded => {}
            RecordOutcome::Replayed => {
                record = self.ledger.lookup(bundle.id, bundle.generation)?.record;
            }
        }
        let ruleset = SandboxRuleset {
            names: &bundle.names,
            lease: bundle.leases.guest,
            guest_mac: bundle.macs.guest,
            intent,
            protected: self.profile.protected(),
        }
        .render();
        bundle.namespace.within(move || nft::apply(&ruleset))?;
        let reservations = ingress::reserve(intent.publications())?;
        let dns = DnsPlan::from_intent(intent, bundle.leases.guest.host());
        let launch = LaunchNetwork::new(
            record.vsock_cid,
            record.generation.get(),
            record.guest_mac,
            bundle.leases.guest.guest().octets(),
            bundle.leases.guest.prefix_length(),
            bundle.leases.guest.host().octets(),
            dns.launch_resolver().octets(),
            record.time_sample_nanos,
        )
        .map_err(|_| Error::InvalidState("launch network"))?;
        let activation =
            ActivationChallenge::generate().map_err(|_| Error::InvalidState("activation"))?;
        Ok((record, launch, reservations, activation))
    }
}

fn now_nanos() -> Result<u64, Error> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Kernel {
            step: Step::Clock,
            errno: 0,
        })?;
    u64::try_from(elapsed.as_nanos())
        .ok()
        .filter(|nanos| *nanos != 0)
        .ok_or(Error::Kernel {
            step: Step::Clock,
            errno: 0,
        })
}
