//! The broker's attestation of one immutable restore input, as one control packet.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use soma_kvm::snapshot::{ArtifactAttestation, AttestedArtifact};

use super::super::{ControlError, Request};

/// Which input, and the broker's record of it.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub type Record = (AttestedArtifact, ArtifactAttestation);

/// `artifact_attestation <memory|root|head> <digest> <stamp fields>`; there is no volume form.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(in crate::control) fn encode((kind, record): &Record) -> String {
    format!("artifact_attestation {} {}", kind.token(), record.encode())
}

/// The rest of an `artifact_attestation` packet.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(in crate::control) fn decode<'a>(
    mut tokens: impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let kind = tokens
        .next()
        .and_then(AttestedArtifact::from_token)
        .ok_or(ControlError::InvalidValue("attested artifact"))?;
    ArtifactAttestation::decode(&tokens.collect::<Vec<_>>().join(" "))
        .map(|record| Request::AttestArtifact((kind, record)))
        .ok_or(ControlError::InvalidValue("artifact attestation"))
}

/// A platform with no checkpoint restore has nothing to attest.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(in crate::control) fn decode<'a>(
    _tokens: impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    Err(ControlError::UnknownRequest)
}
