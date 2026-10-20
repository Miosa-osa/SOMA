//! Sterile bundle preparation and atomic Instance assignment.
//!
//! `prepare` builds a namespace, a down TAP with the gateway address, a down veth pair whose
//! peer already sits in the namespace, the host zone and masquerade table, a fully denied
//! sandbox ruleset, and forwarding off.
//! `assign` records ownership in the ledger before it re-renders the ruleset for the admitted
//! intent, reserves ports, and produces the exact launch-page values.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use soma_guest::{ActivationChallenge, LaunchNetwork};

use crate::connect::ConnectPorts;
use crate::{
    AssignmentRecord, BundleId, BundleNames, CleanupGeneration, ConntrackZone, DnsPlan, Error,
    InstanceId, Ipam, LeasePair, Ledger, NetworkIntent, NetworkProfile, OperationId,
    PortReservation, PublishedPort, RecordOutcome, SandboxRuleset, Step, derive_macs, ingress,
    namespace::NetNamespace, nft, release,
};

mod egress;
pub use egress::EgressChange;
mod prepare;
mod types;

impl Broker {
    /// Replaces the guest ports [`Broker::connect_guest`] may dial.
    #[must_use]
    pub fn with_connect_ports(mut self, ports: ConnectPorts) -> Self {
        self.connect_ports = ports;
        self
    }

    pub(crate) const fn connect_ports(&self) -> &ConnectPorts {
        &self.connect_ports
    }
}

pub use types::{AssignFailure, Assigned, SterileBundle};

struct AssignedParts {
    record: AssignmentRecord,
    launch: LaunchNetwork,
    reservations: Vec<PortReservation>,
    published: Vec<PublishedPort>,
    activation: ActivationChallenge,
}

/// The broker state shared by every bundle operation.
#[derive(Debug)]
pub struct Broker {
    profile: NetworkProfile,
    ns_dir: PathBuf,
    ledger: Ledger,
    generation: CleanupGeneration,
    allocator: Mutex<Allocator>,
    prewarm: NetworkIntent,
    connect_ports: ConnectPorts,
}

/// The lease and conntrack-zone counters, behind one lock so concurrent preparation cannot
/// hand two bundles the same addresses.
#[derive(Debug)]
struct Allocator {
    ipam: Ipam,
    next_zone: u16,
    free_zones: Vec<u16>,
}

impl Allocator {
    /// Takes one lease pair and one conntrack zone, or neither.
    fn take(&mut self) -> Result<(LeasePair, ConntrackZone), Error> {
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
        let prewarm = NetworkIntent::default_egress(&profile);
        Ok(Self {
            profile,
            ns_dir: state_dir.join("ns"),
            ledger,
            generation,
            allocator: Mutex::new(Allocator {
                ipam,
                next_zone: 1,
                free_zones: Vec::new(),
            }),
            prewarm,
            connect_ports: ConnectPorts::default(),
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
        self.generation
    }

    /// The intent every prepared bundle's ruleset is already rendered for.
    ///
    /// A claim whose admitted intent renders to the same ruleset skips the kernel commit that
    /// otherwise dominates assignment.
    #[must_use]
    pub const fn prewarm_intent(&self) -> &NetworkIntent {
        &self.prewarm
    }

    /// Hands a verified-released bundle's lease index and conntrack zone back for reuse.
    ///
    /// Only a release whose live inspection found nothing owned may call this, because the next
    /// holder is given the same addresses and zone.
    pub fn recycle(&self, lease_index: u32, zone: ConntrackZone) {
        let mut allocator = self
            .allocator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        allocator.ipam.recycle(lease_index);
        if zone.get() < allocator.next_zone && !allocator.free_zones.contains(&zone.get()) {
            allocator.free_zones.push(zone.get());
        }
    }

    /// Prepares one sterile bundle; any failure tears down what was built.
    ///
    /// # Errors
    ///
    /// Returns the first typed failure after rollback.
    pub fn prepare(&self, id: BundleId) -> Result<SterileBundle, Error> {
        let (leases, zone) = self
            .allocator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()?;
        let names = BundleNames::new(&id.short_hex());
        let short = id.short_hex();
        let namespace = match NetNamespace::create(&self.ns_dir, &short) {
            Ok(namespace) => namespace,
            Err(error) => {
                // Nothing was built, so the addresses and zone are free again.
                self.recycle(leases.guest.index(), zone);
                return Err(error);
            }
        };
        match prepare::build(self, &names, &namespace, leases, zone, id) {
            Ok((tap, ruleset)) => Ok(SterileBundle {
                id,
                generation: self.generation(),
                names,
                namespace,
                tap,
                leases,
                macs: derive_macs(id),
                zone,
                ruleset,
            }),
            Err(error) => {
                drop(namespace);
                let torn =
                    release::teardown(&names, zone, &self.ns_dir.join(&short), None, Vec::new());
                if torn.complete {
                    self.recycle(leases.guest.index(), zone);
                }
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
            Ok(parts) => Ok(Assigned {
                bundle,
                current: intent.clone(),
                record: parts.record,
                launch: parts.launch,
                reservations: parts.reservations,
                published: parts.published,
                activation: Some(parts.activation),
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
        // The host endpoints are taken before any ruleset changes, so a port already owned by
        // something else fails the assignment while the sandbox is still denied.
        let reservations = ingress::reserve(intent.publications())?;
        let published = ingress::publish_all(&reservations, bundle.leases.guest.guest())?;
        let ruleset = SandboxRuleset {
            names: &bundle.names,
            lease: bundle.leases.guest,
            guest_mac: bundle.macs.guest,
            intent,
            protected: self.profile.protected(),
            published: &published,
            connect_ports: self.connect_ports.ranges(),
        }
        .render();
        // A bundle is prepared with the ruleset for the default intent already committed, so the
        // common claim has nothing to commit. The kernel commit waits out an RCU grace period,
        // which is the largest single cost on the request path, so it runs only when this
        // intent renders to something the namespace does not already hold.
        if prepare::ruleset_digest(&ruleset) != bundle.ruleset {
            bundle.namespace.within(move || nft::apply(&ruleset))?;
        }
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
        Ok(AssignedParts {
            record,
            launch,
            reservations,
            published,
            activation,
        })
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
