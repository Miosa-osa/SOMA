use crate::{
    DnsPolicy, EgressPolicy, HostBind, HostPort, NetworkPolicy, PortPublication, TransportProtocol,
};

use super::{
    EffectiveNetwork, EffectivePortPublication, NetworkAttachment, Observation,
    ObservationUnavailable, PortActivationClass,
};

#[test]
fn automatic_port_request_accepts_an_observed_nonzero_allocation() {
    let requested = NetworkPolicy::new(
        EgressPolicy::Unrestricted,
        DnsPolicy::System,
        vec![
            PortPublication::new(
                HostBind::loopback_v4(),
                HostPort::Automatic,
                8_080,
                TransportProtocol::Tcp,
            )
            .expect("request"),
        ],
    )
    .expect("policy");
    let effective = EffectiveNetwork::new(
        Observation::Observed(NetworkAttachment::Attached),
        Observation::Observed(EgressPolicy::Unrestricted),
        Observation::Observed(DnsPolicy::System),
        Observation::Unavailable(ObservationUnavailable::NotVerified),
        Observation::Observed(vec![
            EffectivePortPublication::new(
                HostBind::loopback_v4(),
                49_152,
                8_080,
                TransportProtocol::Tcp,
            )
            .expect("evidence"),
        ]),
        Observation::Observed(PortActivationClass::VerifiedRuntimeRebind),
    )
    .expect("network evidence");

    assert!(effective.matches_request(&requested));
}

#[test]
fn unavailable_ingress_evidence_never_proves_closed_ingress() {
    let effective = EffectiveNetwork::unavailable(ObservationUnavailable::NotVerified);

    assert!(!effective.matches_request(&NetworkPolicy::isolated()));
}
