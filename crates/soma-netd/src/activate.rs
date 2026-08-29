//! Repair-gated activation.
//!
//! The caller presents a [`RepairAttestation`] only after authenticated guest network repair.
//! The broker then verifies the ledger, namespace, links, rulesets, and forwarding state
//! against the assignment before it raises the links, installs the routes, and finally
//! enables forwarding, which is the single step that makes guest traffic flow.

use crate::{Assigned, Cidr, Drift, Error, InstanceId, link, nft, sysctl};

/// The caller's statement that authenticated network repair succeeded for one Instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepairAttestation {
    instance: InstanceId,
}

impl RepairAttestation {
    /// Builds one attestation; the caller is responsible for its truth.
    #[must_use]
    pub const fn authenticated(instance: InstanceId) -> Self {
        Self { instance }
    }
}

/// What activation verified and changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationEvidence {
    /// The links raised, in order.
    pub links_raised: Vec<String>,
    /// The routes installed, in order.
    pub routes: Vec<String>,
    /// Whether forwarding is now enabled inside the sandbox namespace.
    pub forwarding: bool,
}

/// Verifies and activates one assigned bundle.
///
/// # Errors
///
/// Returns [`Error::InvalidState`] for a wrong Instance or an already active bundle,
/// [`Error::Drift`] when kernel state does not match the ledger, or the first kernel failure.
pub fn activate(
    assigned: &mut Assigned,
    attestation: RepairAttestation,
) -> Result<ActivationEvidence, Error> {
    if attestation.instance != assigned.record.instance {
        return Err(Error::InvalidState("attestation instance"));
    }
    if assigned.active {
        return Err(Error::InvalidState("already active"));
    }
    verify(assigned)?;
    let bundle = &assigned.bundle;
    let names = bundle.names.clone();
    let leases = bundle.leases;
    let mut evidence = ActivationEvidence {
        links_raised: Vec::new(),
        routes: Vec::new(),
        forwarding: false,
    };
    let inner_names = names.clone();
    bundle.namespace.within(move || {
        let socket = link::control_socket()?;
        link::set_up(&socket, &inner_names.tap, true)?;
        link::set_up(&socket, &inner_names.sandbox_veth, true)?;
        link::add_route(
            &socket,
            Cidr::V4(std::net::Ipv4Addr::UNSPECIFIED, 0),
            leases.transit.host(),
        )
    })?;
    evidence.links_raised.push(names.tap.clone());
    evidence.links_raised.push(names.sandbox_veth.clone());
    evidence
        .routes
        .push(format!("sandbox default via {}", leases.transit.host()));
    let socket = link::control_socket()?;
    link::set_up(&socket, &names.host_veth, true)?;
    evidence.links_raised.push(names.host_veth.clone());
    link::add_route(&socket, leases.guest.cidr(), leases.transit.guest())?;
    evidence.routes.push(format!(
        "host {} via {}",
        leases.guest.cidr(),
        leases.transit.guest()
    ));
    bundle.namespace.within(|| sysctl::set_forwarding(true))?;
    evidence.forwarding = true;
    assigned.active = true;
    Ok(evidence)
}

fn verify(assigned: &Assigned) -> Result<(), Error> {
    let bundle = &assigned.bundle;
    if !bundle.namespace.path().exists() {
        return Err(Error::Drift(Drift::NamespaceMissing));
    }
    let names = bundle.names.clone();
    bundle.namespace.within(move || {
        let links = link::list_links()?;
        if !links.contains(&names.tap) {
            return Err(Error::Drift(Drift::TapMissing));
        }
        if !links.contains(&names.sandbox_veth) {
            return Err(Error::Drift(Drift::VethMissing));
        }
        let socket = link::control_socket()?;
        if link::is_up(&socket, &names.tap)? || link::is_up(&socket, &names.sandbox_veth)? {
            return Err(Error::Drift(Drift::LinkAlreadyUp));
        }
        if sysctl::forwarding()? {
            return Err(Error::Drift(Drift::ForwardingAlreadyEnabled));
        }
        if !nft::list_tables()?.contains(&names.sandbox_table) {
            return Err(Error::Drift(Drift::RulesetMissing));
        }
        Ok(())
    })?;
    if !link::list_links()?.contains(&bundle.names.host_veth) {
        return Err(Error::Drift(Drift::HostVethMissing));
    }
    if !nft::list_tables()?.contains(&bundle.names.host_table) {
        return Err(Error::Drift(Drift::HostRulesetMissing));
    }
    Ok(())
}
