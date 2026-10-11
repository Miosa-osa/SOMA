//! Single-use, claimant-bound activation.
//!
//! The assignment carries one fresh single-use [`soma_guest::ActivationChallenge`] that the
//! broker delivered only to the peer that claimed it.
//! Activation consumes that challenge and requires a [`soma_guest::ActivationReceipt`] that
//! authenticates against it and against this exact Instance, assignment generation, Launch
//! operation, and admitted intent digest.
//!
//! This is a continuity check on the claiming peer, not guest evidence: the challenge is the
//! broker's own secret, delivered to that peer, so the peer can compute the receipt without a
//! guest session. Nothing here proves that guest repair completed.
//! Only then does the broker verify the ledger, namespace, links, rulesets, and forwarding
//! state, enable forwarding, which is the single step that makes guest traffic flow, and
//! install the publication table, which is the single step that makes a published port
//! reachable.
//! The links and routes are raised when the bundle is prepared, not here: they take the
//! kernel's network configuration lock, and activation is on the path of every create.
//!
//! The challenge is taken before any verification, so one challenge authorizes at most one
//! activation attempt and a replayed receipt can never reach the kernel.
//! The receipt that succeeded is retained on the assignment, so the broker answers a peer that
//! lost its `Activated` reply from that record instead of running activation again.

use soma_guest::{ActivationReceipt, ActivationScope};

use crate::{Assigned, Drift, Error, PublishedPort, link, nft, sysctl};

mod publish;

/// What activation verified and changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationEvidence {
    /// The links raised, in order.
    pub links_raised: Vec<String>,
    /// The routes installed, in order.
    pub routes: Vec<String>,
    /// Whether forwarding is now enabled inside the sandbox namespace.
    pub forwarding: bool,
    /// The published mappings installed, in order, one description per mapping.
    pub published: Vec<String>,
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
    // Verification and the forwarding step share one namespace entry: each entry is a thread
    // and a `setns`, and activation runs on the path of every create.
    verify_and_forward(assigned)?;
    let mut evidence = ActivationEvidence {
        links_raised: Vec::new(),
        routes: Vec::new(),
        forwarding: false,
        published: Vec::new(),
        transcript: *receipt.transcript(),
    };
    // The links and routes were raised when the bundle was prepared, so the one step left that
    // lets guest traffic flow, forwarding, was taken by the entry above.
    evidence.forwarding = true;
    evidence.published = publish::install(assigned)?;
    assigned.activated = Some(*receipt);
    assigned.active = true;
    Ok(evidence)
}

fn verify_and_forward(assigned: &Assigned) -> Result<(), Error> {
    let bundle = &assigned.bundle;
    if !bundle.namespace.path().exists() {
        return Err(Error::Drift(Drift::NamespaceMissing));
    }
    if !link::list_links()?.contains(&bundle.names.host_veth) {
        return Err(Error::Drift(Drift::HostVethMissing));
    }
    if !nft::table_exists(&bundle.names.host_table)? {
        return Err(Error::Drift(Drift::HostRulesetMissing));
    }
    if nft::table_exists(&bundle.names.publication_table)? {
        return Err(Error::Drift(Drift::PublicationRulesetPresent));
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
        if !link::is_up(&socket, &names.tap)? || !link::is_up(&socket, &names.sandbox_veth)? {
            return Err(Error::Drift(Drift::LinkNotRaised));
        }
        if sysctl::forwarding()? {
            return Err(Error::Drift(Drift::ForwardingAlreadyEnabled));
        }
        if !nft::table_exists(&names.sandbox_table)? {
            return Err(Error::Drift(Drift::RulesetMissing));
        }
        // Everything the host side asserts is checked before this entry: the forwarding write
        // below is the step that lets traffic flow, so nothing may fail after it.
        sysctl::set_forwarding(true)
    })?;
    Ok(())
}
