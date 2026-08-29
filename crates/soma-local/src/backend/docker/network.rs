use soma::{
    DnsPolicy, EffectiveNetwork, EgressPolicy, NetworkAttachment, Observation, PortActivationClass,
};

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
