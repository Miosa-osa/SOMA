//! Conservative detection of secret material committed as a literal.
//!
//! Version 1 combines three signals: a name that conventionally carries a credential, a
//! value shaped like a known credential format, and a module declaring the name as one that
//! must arrive through secret delivery.
//! A name marker matches whole underscore-delimited components, so `GITHUB_TOKEN` is a
//! credential carrier while `TOKENIZERS_PARALLELISM` is not, and a name marker alone never
//! rejects a trivial value such as `false` or `0` because such a value cannot be a credential.
//! Value detection also looks inside `/`, `=`, `:`, and whitespace separated segments, so a
//! credential embedded in a path, a `--flag=value` argument, or a sentence is still found.

use crate::module::ModuleSpec;

pub(crate) const SECRET_SOURCE_SCHEME: &str = "secret://";

const SECRET_NAME_MARKERS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "PRIVATE_KEY",
    "API_KEY",
    "APIKEY",
    "ACCESS_KEY",
    "CREDENTIAL",
];

const SECRET_VALUE_PREFIXES: &[&str] = &[
    "sk-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxs-",
    "AIza",
];

/// Whether a name carries a credential marker as one or more whole `_` components.
pub(super) fn secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let components: Vec<&str> = upper.split('_').collect();
    SECRET_NAME_MARKERS.iter().any(|marker| {
        let marker: Vec<&str> = marker.split('_').collect();
        components
            .windows(marker.len())
            .any(|window| window == marker.as_slice())
    })
}

/// A value that cannot be a credential: empty, a boolean word, or a single binary digit.
fn trivial_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || ["true", "false", "0", "1"]
            .iter()
            .any(|word| trimmed.eq_ignore_ascii_case(word))
}

/// Whether one environment literal must be rejected: the name is declared secret by a
/// composed module, the value has a credential shape, or the name conventionally carries a
/// credential and the value is not trivial.
pub(super) fn environment_literal(modules: &[&ModuleSpec], name: &str, value: &str) -> bool {
    let module_secret = modules.iter().any(|module| {
        module
            .secret_environment()
            .iter()
            .any(|secret| secret.as_str() == name)
    });
    module_secret || embedded_secret(value) || (secret_name(name) && !trivial_value(value))
}

/// Whether `value`, or any separator-delimited segment of it, has a credential shape.
pub(super) fn embedded_secret(value: &str) -> bool {
    secret_value(value)
        || value
            .split(|character: char| {
                matches!(character, '/' | '=' | ':') || character.is_whitespace()
            })
            .any(secret_value)
}

fn secret_value(value: &str) -> bool {
    let trimmed = value.trim();
    if SECRET_VALUE_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    {
        return true;
    }
    if trimmed.starts_with("-----BEGIN ") && trimmed.contains("PRIVATE KEY-----") {
        return true;
    }
    if aws_access_key(trimmed) {
        return true;
    }
    json_web_token(trimmed)
}

/// `AKIA` or `ASIA` followed by exactly sixteen uppercase letters or digits.
fn aws_access_key(value: &str) -> bool {
    let Some(rest) = value
        .strip_prefix("AKIA")
        .or_else(|| value.strip_prefix("ASIA"))
    else {
        return false;
    };
    rest.len() == 16
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

/// Three base64url segments starting with the `{"` header prefix.
fn json_web_token(value: &str) -> bool {
    if !value.starts_with("eyJ") {
        return false;
    }
    let segments: Vec<&str> = value.split('.').collect();
    segments.len() == 3
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
}

/// The shape of a secret source: the `secret://` scheme and a non-empty graphic ASCII path.
pub(crate) fn secret_source_shape(source: &str) -> bool {
    source
        .strip_prefix(SECRET_SOURCE_SCHEME)
        .is_some_and(|path| !path.is_empty() && path.bytes().all(|byte| byte.is_ascii_graphic()))
}

/// A secret source must be a reference into a secret store, never a literal, and no path
/// segment of the reference may itself be a credential.
pub(super) fn secret_source(source: &str) -> bool {
    secret_source_shape(source)
        && source
            .strip_prefix(SECRET_SOURCE_SCHEME)
            .is_some_and(|path| !embedded_secret(path))
}
