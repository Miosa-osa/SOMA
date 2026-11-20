use soma::{
    DnsPolicy, EffectiveNetwork, EgressPolicy, NetworkAttachment, NetworkPolicy, Observation,
    PortActivationClass,
};

/// The Docker network a launch gives `policy`, or nothing for a policy Docker cannot serve.
pub(crate) fn network_mode(policy: &NetworkPolicy) -> Option<&'static str> {
    match (policy.egress(), policy.dns(), policy.published_ports()) {
        (EgressPolicy::Denied, DnsPolicy::Denied, []) => Some("none"),
        (EgressPolicy::Unrestricted, DnsPolicy::System, []) => Some("bridge"),
        _ => None,
    }
}

pub(super) fn effective_network(mode: &str) -> EffectiveNetwork {
    let attached = mode != "none";
    EffectiveNetwork::new(
        Observation::Observed(if attached {
            NetworkAttachment::Attached
        } else {
            NetworkAttachment::Detached
        }),
        Observation::Observed(if attached {
            EgressPolicy::Unrestricted
        } else {
            EgressPolicy::Denied
        }),
        Observation::Observed(if attached {
            DnsPolicy::System
        } else {
            DnsPolicy::Denied
        }),
        Observation::Observed(Vec::new()),
        Observation::Observed(Vec::new()),
        Observation::Observed(PortActivationClass::NotApplicable),
    )
    .expect("Docker network observations are canonical")
}
