//! The normalized network envelope and its comparison with the policy ceiling.
//!
//! Authored intent is normalized so that `deny` with explicit destinations and `allowlist`
//! with the same destinations describe one envelope, destination lists are stored sorted
//! and deduplicated because their order has no effect on the sandbox, and every CIDR is
//! stored in its one canonical text so textual variants of one network share one lock.

use super::{
    cidr::{self, Cidr},
    policy::PolicyCeiling,
    syntax,
};
use crate::{
    error::LockError,
    rejection::{InvalidReason, Rejection},
    schema::{EgressIntent, IngressIntent, MAX_CIDRS, MAX_DOMAINS, MAX_STRING_BYTES, Network},
    wire::{Reader, Writer},
};

/// The effective maximum egress permission of one Template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EgressEnvelope {
    Deny,
    Allowlist {
        domains: Vec<String>,
        cidrs: Vec<String>,
    },
    Unrestricted,
}

impl EgressEnvelope {
    #[must_use]
    pub const fn intent(&self) -> EgressIntent {
        match self {
            Self::Deny => EgressIntent::Deny,
            Self::Allowlist { .. } => EgressIntent::Allowlist,
            Self::Unrestricted => EgressIntent::Unrestricted,
        }
    }
}

/// The maximum network permissions an Instance of this Template may receive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkEnvelope {
    egress: EgressEnvelope,
    ingress: IngressIntent,
}

impl NetworkEnvelope {
    #[must_use]
    pub const fn egress(&self) -> &EgressEnvelope {
        &self.egress
    }

    #[must_use]
    pub const fn ingress(&self) -> IngressIntent {
        self.ingress
    }

    /// Whether outbound traffic to `host` is inside the envelope.
    #[must_use]
    pub fn allows_domain(&self, host: &str) -> bool {
        match &self.egress {
            EgressEnvelope::Deny => false,
            EgressEnvelope::Unrestricted => true,
            EgressEnvelope::Allowlist { domains, .. } => domains
                .iter()
                .any(|pattern| syntax::domain_covers(pattern, host)),
        }
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_u8(self.egress.intent().code());
        if let EgressEnvelope::Allowlist { domains, cidrs } = &self.egress {
            writer.put_strings(domains);
            writer.put_strings(cidrs);
        }
        writer.put_u8(self.ingress.code());
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, LockError> {
        let code = reader.u8()?;
        let egress = match EgressIntent::from_code(code) {
            Some(EgressIntent::Deny) => EgressEnvelope::Deny,
            Some(EgressIntent::Unrestricted) => EgressEnvelope::Unrestricted,
            Some(EgressIntent::Allowlist) => {
                let domains = reader.strings(MAX_DOMAINS, MAX_STRING_BYTES)?;
                let cidrs = reader.strings(MAX_CIDRS, MAX_STRING_BYTES)?;
                if (domains.is_empty() && cidrs.is_empty())
                    || !canonical(&domains)
                    || !canonical(&cidrs)
                    || domains.iter().any(|domain| syntax::domain(domain).is_err())
                    || !cidrs.iter().all(|value| cidr::is_canonical(value))
                {
                    return Err(LockError::InvalidField {
                        field: "network.egress",
                    });
                }
                EgressEnvelope::Allowlist { domains, cidrs }
            }
            None => {
                return Err(LockError::InvalidDiscriminant {
                    field: "network.egress",
                    value: code,
                });
            }
        };
        let code = reader.u8()?;
        let ingress = IngressIntent::from_code(code).ok_or(LockError::InvalidDiscriminant {
            field: "network.ingress",
            value: code,
        })?;
        Ok(Self { egress, ingress })
    }
}

/// Validates authored intent, checks it against the ceiling, and normalizes the envelope.
pub(super) fn envelope(
    network: &Network,
    ceiling: &PolicyCeiling,
) -> Result<NetworkEnvelope, Rejection> {
    for (index, domain) in network.allow_domains.iter().enumerate() {
        syntax::domain(domain).map_err(|reason| invalid("allow_domains", index, reason))?;
    }
    for (index, cidr) in network.allow_cidrs.iter().enumerate() {
        syntax::cidr(cidr).map_err(|reason| invalid("allow_cidrs", index, reason))?;
    }
    let has_destinations = !network.allow_domains.is_empty() || !network.allow_cidrs.is_empty();
    let egress = match network.egress {
        EgressIntent::Unrestricted if has_destinations => {
            let field = if network.allow_domains.is_empty() {
                "allow_cidrs"
            } else {
                "allow_domains"
            };
            return Err(invalid(field, 0, InvalidReason::ContradictoryEgress));
        }
        EgressIntent::Unrestricted => EgressEnvelope::Unrestricted,
        EgressIntent::Allowlist if !has_destinations => {
            return Err(Rejection::InvalidValue {
                module: None,
                field: "network.egress".to_owned(),
                reason: InvalidReason::EmptyAllowlist,
            });
        }
        EgressIntent::Deny if !has_destinations => EgressEnvelope::Deny,
        EgressIntent::Allowlist | EgressIntent::Deny => EgressEnvelope::Allowlist {
            domains: sorted(&network.allow_domains),
            cidrs: cidr::canonical_list(&network.allow_cidrs),
        },
    };
    check_ceiling(network, &egress, ceiling)?;
    Ok(NetworkEnvelope {
        egress,
        ingress: network.ingress,
    })
}

fn check_ceiling(
    network: &Network,
    egress: &EgressEnvelope,
    ceiling: &PolicyCeiling,
) -> Result<(), Rejection> {
    if network.ingress > ceiling.ingress() {
        return Err(exceeds(
            "network.ingress",
            network.ingress.as_str(),
            ceiling.ingress().as_str(),
        ));
    }
    if egress.intent() > ceiling.egress() {
        return Err(exceeds(
            "network.egress",
            egress.intent().as_str(),
            ceiling.egress().as_str(),
        ));
    }
    if !matches!(egress, EgressEnvelope::Allowlist { .. }) {
        return Ok(());
    }
    if let Some(allowed) = ceiling.domains() {
        for (index, domain) in network.allow_domains.iter().enumerate() {
            let covered = allowed
                .iter()
                .any(|pattern| pattern_covers(pattern, domain));
            if !covered {
                return Err(exceeds(
                    &format!("network.allow_domains[{index}]"),
                    domain,
                    &format!("{} permitted domain patterns", allowed.len()),
                ));
            }
        }
    }
    if let Some(allowed) = ceiling.cidrs() {
        let outer: Vec<Cidr> = allowed
            .iter()
            .filter_map(|value| Cidr::parse(value).ok())
            .collect();
        for (index, requested) in network.allow_cidrs.iter().enumerate() {
            // Every value was shape-checked already; anything unparseable is not covered.
            let covered = Cidr::parse(requested)
                .is_ok_and(|inner| outer.iter().any(|outer| outer.contains(&inner)));
            if !covered {
                return Err(exceeds(
                    &format!("network.allow_cidrs[{index}]"),
                    requested,
                    &format!("{} permitted CIDRs", allowed.len()),
                ));
            }
        }
    }
    Ok(())
}

/// Whether a ceiling pattern covers a requested pattern, including wildcard requests.
fn pattern_covers(ceiling: &str, requested: &str) -> bool {
    match requested.strip_prefix("*.") {
        Some(suffix) => ceiling
            .strip_prefix("*.")
            .is_some_and(|allowed| allowed == suffix || syntax::domain_covers(ceiling, suffix)),
        None => syntax::domain_covers(ceiling, requested),
    }
}

fn canonical(values: &[String]) -> bool {
    values.is_sorted() && values.windows(2).all(|pair| pair[0] != pair[1])
}

fn sorted(values: &[String]) -> Vec<String> {
    let mut values = values.to_vec();
    values.sort();
    values.dedup();
    values
}

fn invalid(list: &str, index: usize, reason: InvalidReason) -> Rejection {
    Rejection::InvalidValue {
        module: None,
        field: format!("network.{list}[{index}]"),
        reason,
    }
}

fn exceeds(field: &str, requested: &str, ceiling: &str) -> Rejection {
    Rejection::NetworkExceedsCeiling {
        field: field.to_owned(),
        requested: requested.to_owned(),
        ceiling: ceiling.to_owned(),
    }
}
