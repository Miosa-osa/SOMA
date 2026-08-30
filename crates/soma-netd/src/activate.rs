//! Repair-gated activation.
//!
//! The assignment carries one fresh single-use [`soma_guest::ActivationChallenge`] that the
//! broker delivered only to the peer that claimed it.
//! Activation consumes that challenge and requires a [`soma_guest::ActivationReceipt`] minted
//! by the repaired authenticated guest session, bound to this exact Instance, assignment
//! generation, Launch operation, admitted intent digest, and session transcript.
//! Only then does the broker verify the ledger, namespace, links, rulesets, and forwarding
//! state, raise the links, install the routes, and finally enable forwarding, which is the
//! single step that makes guest traffic flow.
//!
//! The challenge is taken before any verification, so one challenge authorizes at most one
//! activation attempt and a replayed receipt can never reach the kernel.
//! The receipt that succeeded is retained on the assignment, so the broker answers a peer that
//! lost its `Activated` reply from that record instead of running activation again.

use soma_guest::{ActivationReceipt, ActivationScope};

use crate::{Assigned, Cidr, Drift, Error, link, nft, sysctl};

/// What activation verified and changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationEvidence {
    /// The links raised, in order.
    pub links_raised: Vec<String>,
    /// The routes installed, in order.
    pub routes: Vec<String>,
    /// Whether forwarding is now enabled inside the sandbox namespace.
    pub forwarding: bool,
    /// The authenticated guest-session transcript the consumed receipt was minted from.
    pub transcript: [u8; 32],
}

/// Consumes the assignment's activation challenge, then verifies and activates the bundle.
///
/// # Errors
///
/// Returns [`Error::Unauthorized`] when the challenge is already spent or the receipt does not
/// authenticate for this exact assignment, [`Error::Drift`] when kernel state does not match
/// the ledger, or the first kernel failure.
pub fn activate(
    assigned: &mut Assigned,
    receipt: &ActivationReceipt,
) -> Result<ActivationEvidence, Error> {
    let challenge = assigned
        .activation
        .take()
        .ok_or(Error::Unauthorized("activation challenge spent"))?;
    let scope = ActivationScope::new(
        *assigned.record.instance.as_bytes(),
        *assigned.record.operation.as_bytes(),
        assigned.record.generation.get(),
        assigned.record.intent_digest.0,
    )
    .map_err(|_| Error::Unauthorized("activation scope"))?;
    challenge
        .verify(&scope, receipt)
        .map_err(|_| Error::Unauthorized("activation receipt"))?;
    verify(assigned)?;
    let bundle = &assigned.bundle;
    let names = bundle.names.clone();
    let leases = bundle.leases;
    let mut evidence = ActivationEvidence {
        links_raised: Vec::new(),
        routes: Vec::new(),
        forwarding: false,
        transcript: *receipt.transcript(),
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
    assigned.activated = Some(*receipt);
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
        if !nft::table_exists(&names.sandbox_table)? {
            return Err(Error::Drift(Drift::RulesetMissing));
        }
        Ok(())
    })?;
    if !link::list_links()?.contains(&bundle.names.host_veth) {
        return Err(Error::Drift(Drift::HostVethMissing));
    }
    if !nft::table_exists(&bundle.names.host_table)? {
        return Err(Error::Drift(Drift::HostRulesetMissing));
    }
    Ok(())
}
