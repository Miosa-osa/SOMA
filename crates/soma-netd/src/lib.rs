//! `soma-netd`: the privileged Linux network broker for SOMA sterile network bundles.
//!
//! The crate owns network namespaces, TAP and veth devices, IPAM, routes, nftables rulesets,
//! conntrack zones, resolver policy, ingress reservations, the durable ownership ledger,
//! repair-gated activation, idempotent release, and reconciliation.
//! The unprivileged VMM receives exactly one TAP descriptor through `SOCK_SEQPACKET` plus
//! `SCM_RIGHTS`.
//!
//! Intent, profile, protected-set, IPAM, DNS, firewall text, ledger, and protocol types are
//! portable; every kernel mechanism is Linux-only and fails closed elsewhere.

mod authority;
mod cidr;
mod dns;
mod error;
mod firewall;
mod ids;
mod ingress;
mod intent;
mod ipam;
mod ledger;
mod profile;
mod protected;
mod protocol;
mod transfer;

#[cfg(target_os = "linux")]
mod activate;
#[cfg(target_os = "linux")]
mod bundle;
#[cfg(target_os = "linux")]
mod daemon;
#[cfg(target_os = "linux")]
mod link;
#[cfg(target_os = "linux")]
mod listener;
#[cfg(target_os = "linux")]
mod namespace;
#[cfg(target_os = "linux")]
mod netlink;
#[cfg(target_os = "linux")]
mod nft;
#[cfg(target_os = "linux")]
mod reconcile;
#[cfg(target_os = "linux")]
mod release;
#[cfg(target_os = "linux")]
mod sysctl;
#[cfg(target_os = "linux")]
mod tap;

pub use authority::{Capability, ControlAuthority, PeerIdentity};
pub use cidr::Cidr;
pub use dns::DnsPlan;
pub use error::{Drift, Error, IntentRejection, Step, Tool, TransferRejection};
pub use firewall::{BundleNames, SandboxRuleset, host::HostRuleset, mac_text};
pub use ids::{BundleId, CleanupGeneration, ConntrackZone, InstanceId, OperationId};
pub use ingress::{
    PortReservation, attach_proxy, describe as describe_reservation, publish, reserve,
};
pub use intent::{EgressClass, IntentDigest, MAX_ENCODED_INTENT, NetworkIntent};
pub use ipam::{Ipam, Lease, LeasePair, MacPair, SubnetPlan, derive_macs};
pub use ledger::{AssignmentRecord, Ledger, LedgerEntry, MAX_RECORD, RecordOutcome};
pub use profile::{InterfaceName, NetworkProfile, ProfileDigest};
pub use protected::{CERTIFIED_DEFAULT, ProtectedDestination, ProtectedReason, ProtectedSet};
pub use protocol::{MAX_FRAME, Reply, Request, error_code};
pub use transfer::{MAX_HEADER, TransferHeader};

#[cfg(target_os = "linux")]
pub use activate::{ActivationEvidence, activate};
#[cfg(target_os = "linux")]
pub use bundle::{AssignFailure, Assigned, Broker, SterileBundle};
#[cfg(target_os = "linux")]
pub use daemon::serve;
#[cfg(target_os = "linux")]
pub use listener::{Accepted, ControlListener, IDLE_TIMEOUT, broker_owner};
#[cfg(target_os = "linux")]
pub use namespace::{NetNamespace, Unpinned};
#[cfg(target_os = "linux")]
pub use reconcile::{Disposition, ReconcileReport, reconcile};
#[cfg(target_os = "linux")]
pub use release::{ReleaseEvidence, StepResult, release, release_record, release_sterile};
#[cfg(target_os = "linux")]
pub use transfer::{receive_tap, send_tap, seqpacket_pair};
