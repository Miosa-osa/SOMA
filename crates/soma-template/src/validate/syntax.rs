//! Shape rules for domains, CIDRs, users, modes, and guest paths.

use std::net::IpAddr;

use crate::{module::GuestPath, rejection::InvalidReason};

const MAX_DOMAIN_BYTES: usize = 253;
const MAX_LABEL_BYTES: usize = 63;
const MAX_USER_BYTES: usize = 32;
/// One year in seconds; a longer idle or maximum lifetime is treated as a typo.
pub(crate) const MAX_TIMEOUT_SECONDS: u64 = 365 * 24 * 60 * 60;

/// A lowercase DNS name, optionally prefixed by `*.` to allow every subdomain.
///
/// The final label must not be all digits (RFC 3696 section 2), which also keeps an IPv4
/// literal such as `169.254.169.254` from passing as a domain and bypassing a CIDR ceiling;
/// an address is declared under `allow_cidrs` with an explicit prefix.
pub(crate) fn domain(value: &str) -> Result<(), InvalidReason> {
    let name = value.strip_prefix("*.").unwrap_or(value);
    if name.is_empty() || value.len() > MAX_DOMAIN_BYTES || name.ends_with('.') {
        return Err(InvalidReason::InvalidDomain);
    }
    let labels_valid = name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= MAX_LABEL_BYTES
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    });
    let numeric_final_label = name
        .rsplit('.')
        .next()
        .is_some_and(|label| label.bytes().all(|byte| byte.is_ascii_digit()));
    if !labels_valid || numeric_final_label {
        return Err(InvalidReason::InvalidDomain);
    }
    Ok(())
}

/// Whether `pattern` from an allowlist covers `host`, honoring a `*.` wildcard prefix.
pub(crate) fn domain_covers(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(suffix) => host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.') && prefix.len() > 1),
        None => pattern == host,
    }
}

/// An IPv4 or IPv6 address with an explicit prefix length.
pub(crate) fn cidr(value: &str) -> Result<(), InvalidReason> {
    let (address, prefix) = value.split_once('/').ok_or(InvalidReason::InvalidCidr)?;
    let address: IpAddr = address.parse().map_err(|_| InvalidReason::InvalidCidr)?;
    if prefix.is_empty() || prefix.len() > 3 || !prefix.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InvalidReason::InvalidCidr);
    }
    if prefix.len() > 1 && prefix.starts_with('0') {
        return Err(InvalidReason::InvalidCidr);
    }
    let prefix: u32 = prefix.parse().map_err(|_| InvalidReason::InvalidCidr)?;
    let maximum = if address.is_ipv4() { 32 } else { 128 };
    if prefix > maximum {
        return Err(InvalidReason::InvalidCidr);
    }
    Ok(())
}

/// A portable POSIX user name: a lowercase letter or underscore, then lowercase letters,
/// digits, underscores, or hyphens, at most 32 bytes.
pub(crate) fn user(value: &str) -> Result<(), InvalidReason> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(InvalidReason::InvalidUser);
    };
    if value.len() > MAX_USER_BYTES
        || !(first.is_ascii_lowercase() || first == b'_')
        || !bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
    {
        return Err(InvalidReason::InvalidUser);
    }
    Ok(())
}

/// A secret file mode: owner-readable and without group or other permission bits.
pub(crate) fn secret_mode(mode: u32) -> Result<(), InvalidReason> {
    if mode > 0o777 || mode & 0o077 != 0 || mode & 0o400 == 0 {
        return Err(InvalidReason::InvalidMode);
    }
    Ok(())
}

pub(crate) fn absolute_path(value: &str) -> Result<GuestPath, InvalidReason> {
    use crate::module::PathError;
    GuestPath::parse(value).map_err(|error| match error {
        PathError::Empty => InvalidReason::Empty,
        PathError::NotAbsolute => InvalidReason::NotAbsolutePath,
        PathError::NotNormalized | PathError::TooLong => InvalidReason::NotNormalizedPath,
        PathError::ForbiddenCharacter => InvalidReason::ForbiddenCharacter,
    })
}

pub(crate) fn port(value: u16) -> Result<(), InvalidReason> {
    if value == 0 {
        return Err(InvalidReason::InvalidPort);
    }
    Ok(())
}

pub(crate) fn timeout(value: u64) -> Result<(), InvalidReason> {
    if value == 0 || value > MAX_TIMEOUT_SECONDS {
        return Err(InvalidReason::InvalidTimeout);
    }
    Ok(())
}
