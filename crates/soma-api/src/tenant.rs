use crate::envelope::ApiError;

/// The header that carries caller identity until the tenancy decision record exists.
pub const TENANT_HEADER: &str = "x-soma-tenant";

/// Maximum accepted tenant identifier length, chosen to match the facade's bounded identifiers.
pub const MAX_TENANT_BYTES: usize = 64;

/// The identity a request acts under.
///
/// This is a value the service carries but does not yet trust. It is not proof of anything: no
/// signature, token, or directory lookup stands behind it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TenantId(String);

impl TenantId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// TENANCY SEAM. This is the single place where caller identity is established, and it is
/// deliberately unfinished.
///
/// SOMA has no tenancy decision record yet: `docs/adr/` stops at 0033 and nothing there defines
/// how a caller proves who it is. Rather than invent a token format, a signing scheme, or a key
/// store that a later decision would have to undo, this function reads one header, validates only
/// its shape, and refuses every request that omits it. Replace the body of this function, and
/// nothing else in the service, once the tenancy decision lands.
///
/// It fails closed: no header means no request, so the service can never be deployed in a state
/// where it silently serves anonymous callers because authentication was "not wired up yet".
///
/// # Errors
///
/// Returns a 401 refusal when the header is absent, and a 400 refusal when it is present but
/// outside the accepted shape.
pub fn identify(header_value: Option<&str>) -> Result<TenantId, ApiError> {
    let Some(value) = header_value else {
        return Err(ApiError::new(
            401,
            "identity_required",
            "every request must carry a tenant identity in the x-soma-tenant header",
            false,
        ));
    };
    if !is_acceptable(value) {
        return Err(ApiError::new(
            400,
            "invalid_identity",
            "the tenant identity must be 1 to 64 lowercase alphanumeric or hyphen bytes",
            false,
        ));
    }
    Ok(TenantId(value.to_owned()))
}

/// Accepts the same conservative grammar the facade uses for machine names.
///
/// A narrow grammar is chosen because this value will eventually be a lookup key, and a value
/// that cannot contain a separator, a quote, or a control byte cannot smuggle structure into
/// whatever store later resolves it.
fn is_acceptable(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_TENANT_BYTES
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}
