use serde::{Serialize, Serializer, ser::SerializeMap as _};
use soma_version::Protocol;

/// What `soma version` reports: the release, the build identity, and the contracts it speaks.
///
/// Every field after `version` and before `envelope_schema` was added to `soma.cli.v1` without
/// changing an earlier one, so a client that read the first report still reads this one.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VersionReport {
    pub version: &'static str,
    pub commit: &'static str,
    pub commit_date: &'static str,
    pub source: &'static str,
    pub profile: &'static str,
    pub target: &'static str,
    pub protocols: ProtocolVersions,
    pub envelope_schema: &'static str,
    pub production_ready: bool,
    pub macos_development_lifecycle: CapabilityState,
    pub native_kvm_lifecycle: CapabilityState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Compiled,
    Unavailable,
}

/// The protocols this build speaks, rendered as one object from name to version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProtocolVersions(pub &'static [Protocol]);

impl Serialize for ProtocolVersions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for protocol in self.0 {
            map.serialize_entry(protocol.name, &protocol.version)?;
        }
        map.end()
    }
}
