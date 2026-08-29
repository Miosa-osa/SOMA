//! Organization policy ceilings supplied to validation.
//!
//! The ceiling is an input to the lock: the same Template validated against a different
//! ceiling is a different set of selected inputs.

use soma::OciPlatform;

use super::{cidr, syntax};
use crate::{
    error::{BoundError, LockError},
    schema::{EgressIntent, IngressIntent, MAX_CIDRS, MAX_DOMAINS, MAX_STRING_BYTES},
    wire::{Reader, Writer},
};

/// The widest network permissions an organization allows a Template to request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyCeiling {
    egress: EgressIntent,
    domains: Option<Vec<String>>,
    cidrs: Option<Vec<String>>,
    ingress: IngressIntent,
}

impl PolicyCeiling {
    /// A ceiling that permits every network intent.
    #[must_use]
    pub const fn unrestricted() -> Self {
        Self::new(EgressIntent::Unrestricted, IngressIntent::Unrestricted)
    }

    /// A ceiling that permits no egress and no ingress.
    #[must_use]
    pub const fn deny_all() -> Self {
        Self::new(EgressIntent::Deny, IngressIntent::Deny)
    }

    /// A ceiling with the given maximum intents and no destination restriction.
    #[must_use]
    pub const fn new(egress: EgressIntent, ingress: IngressIntent) -> Self {
        Self {
            egress,
            domains: None,
            cidrs: None,
            ingress,
        }
    }

    /// Restricts allowlist egress to the given domain patterns, stored sorted and
    /// deduplicated.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError::TooMany`] above [`MAX_DOMAINS`] entries and
    /// [`BoundError::InvalidShape`] for a pattern that is not a lowercase domain.
    pub fn with_domains(mut self, domains: &[&str]) -> Result<Self, BoundError> {
        if domains.len() > MAX_DOMAINS {
            return Err(BoundError::TooMany {
                field: "ceiling.domains".to_owned(),
                maximum: MAX_DOMAINS,
            });
        }
        for (index, domain) in domains.iter().enumerate() {
            syntax::domain(domain).map_err(|_| BoundError::InvalidShape {
                field: format!("ceiling.domains[{index}]"),
            })?;
        }
        self.domains = Some(sorted(domains));
        Ok(self)
    }

    /// Restricts allowlist egress to the given CIDRs, stored in canonical text, sorted, and
    /// deduplicated.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError::TooMany`] above [`MAX_CIDRS`] entries and
    /// [`BoundError::InvalidShape`] for a malformed CIDR or one with a host bit set.
    pub fn with_cidrs(mut self, cidrs: &[&str]) -> Result<Self, BoundError> {
        if cidrs.len() > MAX_CIDRS {
            return Err(BoundError::TooMany {
                field: "ceiling.cidrs".to_owned(),
                maximum: MAX_CIDRS,
            });
        }
        let mut canonical = Vec::with_capacity(cidrs.len());
        for (index, value) in cidrs.iter().enumerate() {
            let cidr = cidr::Cidr::parse(value).map_err(|_| BoundError::InvalidShape {
                field: format!("ceiling.cidrs[{index}]"),
            })?;
            canonical.push(cidr.canonical());
        }
        canonical.sort();
        canonical.dedup();
        self.cidrs = Some(canonical);
        Ok(self)
    }

    #[must_use]
    pub const fn egress(&self) -> EgressIntent {
        self.egress
    }

    #[must_use]
    pub const fn ingress(&self) -> IngressIntent {
        self.ingress
    }

    /// The permitted domain patterns, or `None` when any domain is permitted.
    #[must_use]
    pub fn domains(&self) -> Option<&[String]> {
        self.domains.as_deref()
    }

    /// The permitted CIDRs, or `None` when any CIDR is permitted.
    #[must_use]
    pub fn cidrs(&self) -> Option<&[String]> {
        self.cidrs.as_deref()
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_u8(self.egress.code());
        put_optional_list(writer, self.domains.as_deref());
        put_optional_list(writer, self.cidrs.as_deref());
        writer.put_u8(self.ingress.code());
    }

    pub(crate) fn decode(reader: &mut Reader<'_>) -> Result<Self, LockError> {
        let egress = reader.u8()?;
        let egress = EgressIntent::from_code(egress).ok_or(LockError::InvalidDiscriminant {
            field: "ceiling.egress",
            value: egress,
        })?;
        let domains = optional_list(reader, MAX_DOMAINS, |domain| syntax::domain(domain).is_ok())?;
        let cidrs = optional_list(reader, MAX_CIDRS, cidr::is_canonical)?;
        let ingress = reader.u8()?;
        let ingress = IngressIntent::from_code(ingress).ok_or(LockError::InvalidDiscriminant {
            field: "ceiling.ingress",
            value: ingress,
        })?;
        Ok(Self {
            egress,
            domains,
            cidrs,
            ingress,
        })
    }
}

pub(crate) fn platform_key(platform: &OciPlatform) -> String {
    match platform.variant() {
        Some(variant) => format!(
            "{}/{}/{variant}",
            platform.operating_system(),
            platform.architecture()
        ),
        None => format!(
            "{}/{}",
            platform.operating_system(),
            platform.architecture()
        ),
    }
}

pub(crate) fn sorted(values: &[&str]) -> Vec<String> {
    let mut values: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
    values.sort();
    values.dedup();
    values
}

fn put_optional_list(writer: &mut Writer, values: Option<&[String]>) {
    writer.put_presence(values.is_some());
    if let Some(values) = values {
        writer.put_strings(values);
    }
}

/// Reads an optional list that must be sorted, unique, and made of `valid` entries.
fn optional_list(
    reader: &mut Reader<'_>,
    bound: usize,
    valid: fn(&str) -> bool,
) -> Result<Option<Vec<String>>, LockError> {
    if reader.presence()? {
        let values = reader.strings(bound, MAX_STRING_BYTES)?;
        if !values.is_sorted()
            || values.windows(2).any(|pair| pair[0] == pair[1])
            || !values.iter().all(|value| valid(value))
        {
            return Err(LockError::InvalidField {
                field: "ceiling.list",
            });
        }
        Ok(Some(values))
    } else {
        Ok(None)
    }
}
