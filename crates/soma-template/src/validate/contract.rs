//! Environment, secret, and required-environment contract checks.

use std::collections::BTreeSet;

use super::{checks::invalid, network::NetworkEnvelope, secret, syntax};
use crate::{
    compose::Composition,
    lock::{LockedEnvironment, LockedSecret, MAX_LOCK_ENVIRONMENT},
    module::EnvironmentName,
    rejection::{InvalidReason, Rejection},
    schema::{DEFAULT_SECRET_FILE_MODE, SecretDelivery, Template},
};

/// The effective environment contract: Template literals and Launch-required names, plus
/// every module seal, sorted by name because names are unique and order has no effect.
///
/// A Template entry that restates a sealed name is dropped in favour of the seal, which
/// composition already proved carries the same value, so the lock records the sealing module
/// and the bytes are identical whether or not the Template repeats the value.
pub(super) fn environment(
    template: &Template,
    composition: &Composition<'_>,
) -> Result<Vec<LockedEnvironment>, Rejection> {
    let mut names = BTreeSet::new();
    let mut locked = Vec::new();
    for (index, entry) in template.environment().iter().enumerate() {
        EnvironmentName::parse(&entry.name).map_err(|_| {
            invalid(
                format!("environment[{index}].name"),
                InvalidReason::ForbiddenCharacter,
            )
        })?;
        if !names.insert(entry.name.as_str()) {
            return Err(invalid(
                format!("environment[{index}].name"),
                InvalidReason::Duplicate,
            ));
        }
        if let Some(value) = &entry.value
            && secret::environment_literal(&composition.modules, &entry.name, value)
        {
            return Err(Rejection::SecretLiteral {
                module: None,
                field: format!("environment[{index}].value"),
                name: entry.name.clone(),
            });
        }
        let sealed = composition
            .sealed
            .iter()
            .any(|seal| seal.name.as_str() == entry.name);
        if sealed {
            continue;
        }
        locked.push(LockedEnvironment {
            name: entry.name.clone(),
            value: entry.value.clone(),
            sealed_by: None,
        });
    }
    for seal in &composition.sealed {
        locked.push(LockedEnvironment {
            name: seal.name.as_str().to_owned(),
            value: Some(seal.value.clone()),
            sealed_by: Some(seal.module.clone()),
        });
    }
    if locked.len() > MAX_LOCK_ENVIRONMENT {
        return Err(invalid(
            "environment".to_owned(),
            InvalidReason::ExceedsMaximum {
                maximum: MAX_LOCK_ENVIRONMENT as u64,
            },
        ));
    }
    locked.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(locked)
}

/// The secret references, sorted by their unique names.
pub(super) fn secrets(
    template: &Template,
    envelope: &NetworkEnvelope,
) -> Result<Vec<LockedSecret>, Rejection> {
    let mut names = BTreeSet::new();
    let mut locked = Vec::new();
    for (index, secret) in template.secrets().iter().enumerate() {
        EnvironmentName::parse(&secret.name).map_err(|_| {
            invalid(
                format!("secrets[{index}].name"),
                InvalidReason::ForbiddenCharacter,
            )
        })?;
        if !names.insert(secret.name.as_str()) {
            return Err(invalid(
                format!("secrets[{index}].name"),
                InvalidReason::Duplicate,
            ));
        }
        if !secret::secret_source(&secret.source) {
            return Err(Rejection::SecretLiteral {
                module: None,
                field: format!("secrets[{index}].source"),
                name: secret.name.clone(),
            });
        }
        let scope = scope(index, secret, envelope)?;
        if secret::embedded_secret(&scope) {
            return Err(Rejection::SecretLiteral {
                module: None,
                field: format!("secrets[{index}].scope"),
                name: secret.name.clone(),
            });
        }
        let mode_field = format!("secrets[{index}].mode");
        let mode = match (secret.delivery, secret.mode) {
            (SecretDelivery::File, Some(mode)) => {
                syntax::secret_mode(mode).map_err(|reason| invalid(mode_field, reason))?;
                Some(mode)
            }
            (SecretDelivery::File, None) => Some(DEFAULT_SECRET_FILE_MODE),
            (_, Some(_)) => return Err(invalid(mode_field, InvalidReason::InvalidMode)),
            (_, None) => None,
        };
        locked.push(LockedSecret {
            name: secret.name.clone(),
            source: secret.source.clone(),
            delivery: secret.delivery,
            scope,
            mode,
        });
    }
    locked.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(locked)
}

fn scope(
    index: usize,
    secret: &crate::schema::SecretReference,
    envelope: &NetworkEnvelope,
) -> Result<String, Rejection> {
    let field = format!("secrets[{index}].scope");
    match (secret.delivery, secret.scope.as_deref()) {
        (SecretDelivery::Environment, None) => Ok(secret.name.clone()),
        (SecretDelivery::Environment, Some(scope)) => {
            EnvironmentName::parse(scope)
                .map_err(|_| invalid(field, InvalidReason::ForbiddenCharacter))?;
            Ok(scope.to_owned())
        }
        (SecretDelivery::File | SecretDelivery::EgressProxy, None) => {
            Err(Rejection::SecretWithoutScope {
                field,
                name: secret.name.clone(),
            })
        }
        (SecretDelivery::File, Some(scope)) => Ok(syntax::absolute_path(scope)
            .map_err(|reason| invalid(field, reason))?
            .as_str()
            .to_owned()),
        (SecretDelivery::EgressProxy, Some(scope)) => {
            syntax::domain(scope).map_err(|reason| invalid(field.clone(), reason))?;
            if !envelope.allows_domain(scope) {
                return Err(invalid(field, InvalidReason::DestinationNotAllowed));
            }
            Ok(scope.to_owned())
        }
    }
}

pub(super) fn required_environment(
    composition: &Composition<'_>,
    environment: &[LockedEnvironment],
    secrets: &[LockedSecret],
) -> Result<(), Rejection> {
    let provided = |name: &str| {
        environment.iter().any(|entry| entry.name == name)
            || secrets.iter().any(|secret| {
                secret.delivery == SecretDelivery::Environment && secret.scope == name
            })
    };
    for module in &composition.modules {
        for (index, name) in module.required_environment().iter().enumerate() {
            if !provided(name.as_str()) {
                return Err(Rejection::MissingRequiredEnvironment {
                    module: module.identity().clone(),
                    field: format!("required_environment[{index}]"),
                    name: name.as_str().to_owned(),
                });
            }
        }
    }
    Ok(())
}
