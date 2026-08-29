use crate::{DnsPolicy, EgressPolicy, Observation, PortPublication};

use super::{EffectivePortPublication, PortActivationClass};

pub(super) fn egress_matches(
    observed: &Observation<EgressPolicy>,
    requested: EgressPolicy,
) -> bool {
    requested == EgressPolicy::Unspecified || observed == &Observation::Observed(requested)
}

pub(super) fn dns_matches(observed: &Observation<DnsPolicy>, requested: &DnsPolicy) -> bool {
    requested == &DnsPolicy::Unspecified || observed == &Observation::Observed(requested.clone())
}

pub(super) fn publications_match(
    observed: &Observation<Vec<EffectivePortPublication>>,
    requested: &[PortPublication],
) -> bool {
    let Observation::Observed(observed) = observed else {
        return false;
    };
    observed.len() == requested.len()
        && requested.iter().all(|request| {
            observed.iter().any(|effective| {
                effective.bind() == request.bind()
                    && effective.guest_port() == request.guest_port()
                    && effective.protocol() == request.protocol()
                    && request
                        .host_port()
                        .requested()
                        .is_none_or(|port| effective.host_port() == port)
            })
        })
}

pub(super) fn requested_activation_matches(
    observed: &Observation<PortActivationClass>,
    requested: &[PortPublication],
) -> bool {
    if requested.is_empty() {
        return observed == &Observation::Observed(PortActivationClass::NotApplicable);
    }
    matches!(
        observed,
        Observation::Observed(
            PortActivationClass::AtomicSocketHandoff | PortActivationClass::VerifiedRuntimeRebind
        )
    )
}

pub(super) fn activation_matches_publications(
    publications: &Observation<Vec<EffectivePortPublication>>,
    activation: &Observation<PortActivationClass>,
) -> bool {
    match (publications, activation) {
        (
            Observation::Observed(values),
            Observation::Observed(PortActivationClass::NotApplicable),
        ) => values.is_empty(),
        (
            Observation::Observed(values),
            Observation::Observed(
                PortActivationClass::AtomicSocketHandoff
                | PortActivationClass::VerifiedRuntimeRebind,
            ),
        ) => !values.is_empty(),
        (Observation::Unavailable(_), Observation::Unavailable(_)) => true,
        _ => false,
    }
}

pub(super) fn endpoint_collision(publications: &[EffectivePortPublication]) -> bool {
    publications.iter().enumerate().any(|(index, left)| {
        publications[index + 1..].iter().any(|right| {
            left.host_port() == right.host_port()
                && left.protocol() == right.protocol()
                && left.bind().conflicts(right.bind())
        })
    })
}
