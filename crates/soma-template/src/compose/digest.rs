//! The content digest of one composed selection: everything the Template and the module
//! registry determine before any external input is consulted.
//!
//! The projection binds the platform, the composed module order, the effective command with
//! defaults applied, resources, the normalized network intent, lifecycle, the effective
//! environment contract, and the secret references, each in canonical form, so two documents
//! that compose to one selection share one digest whatever their authored spelling.
//! It excludes `name`, `description`, the mutable `workload.image` text, TOML layout, and
//! every external input: the resolved digest, the policy ceiling, and the Backend.

use std::collections::BTreeMap;

use sha2::{Digest as _, Sha256};

use super::Composition;
use crate::{
    module::digest::put_command,
    schema::{DEFAULT_SECRET_FILE_MODE, EgressIntent, Network, SecretDelivery, Template},
    validate::cidr,
    wire::Writer,
};

const MAGIC: &[u8; 8] = b"SOMATMPL";
const PROJECTION_VERSION: u16 = 2;

pub(super) fn content_digest(template: &Template, composition: &Composition<'_>) -> [u8; 32] {
    let mut writer = Writer::with_capacity(1024);
    writer.put_bytes(MAGIC);
    writer.put_u16(PROJECTION_VERSION);
    let platform = template.workload().platform();
    writer.put_string(platform.operating_system());
    writer.put_string(platform.architecture());
    writer.put_optional_string(platform.variant());
    writer.put_count(composition.modules.len());
    for module in &composition.modules {
        writer.put_string(&module.identity().to_string());
    }
    put_command(&mut writer, &composition.command);
    let resources = template.resources();
    writer.put_u32(resources.vcpus);
    writer.put_u64(resources.memory_mib);
    writer.put_u64(resources.writable_storage_mib);
    let network = template.network();
    writer.put_u8(logical_egress(network).code());
    writer.put_strings(&sorted(&network.allow_domains));
    writer.put_strings(&cidr::canonical_list(&network.allow_cidrs));
    writer.put_u8(network.ingress.code());
    let lifecycle = template.lifecycle();
    writer.put_u64(lifecycle.idle_timeout_seconds);
    writer.put_u64(lifecycle.maximum_lifetime_seconds);
    writer.put_u8(lifecycle.on_idle.code());
    let environment = effective_environment(template, composition);
    writer.put_count(environment.len());
    for (name, value) in &environment {
        writer.put_string(name);
        writer.put_optional_string(value.as_deref());
    }
    let mut secrets: Vec<_> = template.secrets().iter().collect();
    secrets.sort_by(|left, right| left.name.cmp(&right.name));
    writer.put_count(secrets.len());
    for secret in secrets {
        writer.put_string(&secret.name);
        writer.put_string(&secret.source);
        writer.put_u8(secret.delivery.code());
        let scope = match (secret.delivery, secret.scope.as_deref()) {
            (SecretDelivery::Environment, None) => Some(secret.name.as_str()),
            (_, scope) => scope,
        };
        writer.put_optional_string(scope);
        let mode = match (secret.delivery, secret.mode) {
            (SecretDelivery::File, None) => Some(DEFAULT_SECRET_FILE_MODE),
            (_, mode) => mode,
        };
        writer.put_presence(mode.is_some());
        if let Some(mode) = mode {
            writer.put_u32(mode);
        }
    }
    Sha256::digest(writer.finish()).into()
}

/// The Template entries and module seals as one name-keyed contract.
///
/// Composition already rejected a Template entry that disagrees with a seal, so a restated
/// seal is the same pair either way; a duplicate Template name is rejected by validation and
/// only needs a deterministic projection here.
fn effective_environment(
    template: &Template,
    composition: &Composition<'_>,
) -> Vec<(String, Option<String>)> {
    let mut entries: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    for entry in template.environment() {
        entries.entry(&entry.name).or_insert(entry.value.as_deref());
    }
    for seal in &composition.sealed {
        entries.insert(seal.name.as_str(), Some(&seal.value));
    }
    entries
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.map(str::to_owned)))
        .collect()
}

/// `deny` with explicit destinations is the same selection as `allowlist` with them.
fn logical_egress(network: &Network) -> EgressIntent {
    let has_destinations = !network.allow_domains.is_empty() || !network.allow_cidrs.is_empty();
    match network.egress {
        EgressIntent::Deny if has_destinations => EgressIntent::Allowlist,
        intent => intent,
    }
}

fn sorted(values: &[String]) -> Vec<String> {
    let mut values = values.to_vec();
    values.sort();
    values.dedup();
    values
}
