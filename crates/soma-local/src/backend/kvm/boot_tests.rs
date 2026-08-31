//! What the launch inputs must bind, and what they must never share.

use super::*;

use crate::backend::kvm::identity::FIRST_GUEST_CID;

/// The launch page must name the identifier the machine was actually built with.
///
/// This is the defect that broke every restored sandbox: the machine was given an identifier
/// derived from the Instance while the launch page still named a constant. The guest agent checks
/// one against the other and refuses the session when they disagree, so a correctly built machine
/// reached its repair point and then could form no session at all. Comparing the two values is
/// the whole test, because nothing else in the launch inputs relates them.
#[test]
fn the_launch_page_names_the_machine_s_own_context_identifier() {
    let instance = InstanceId::new("89db112753324c3e890ef78b74381aa5").expect("identity");
    let assigned = LaunchIdentity::derive(&instance)
        .expect("identity")
        .guest_cid;
    let network = link_down_network(assigned).expect("network");
    assert_eq!(network.vsock_cid(), assigned);
    // The identifier this Instance takes is not the constant the page used to carry, so a page
    // built from that constant would have disagreed with the machine.
    assert_ne!(assigned, FIRST_GUEST_CID);
}
