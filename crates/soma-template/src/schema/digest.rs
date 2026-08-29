//! The content projection of a Template document and its SHA-256 digest.
//!
//! The projection binds every content-affecting authored value in schema order.
//! It excludes `name`, `description`, the mutable `workload.image` text, and TOML layout.

use sha2::{Digest as _, Sha256};

use super::Template;
use crate::{module::digest::put_command, wire::Writer};

const MAGIC: &[u8; 8] = b"SOMATMPL";
const PROJECTION_VERSION: u16 = 1;

pub(super) fn content_digest(template: &Template) -> [u8; 32] {
    let mut writer = Writer::with_capacity(1024);
    writer.put_bytes(MAGIC);
    writer.put_u16(PROJECTION_VERSION);
    let platform = template.workload().platform();
    writer.put_string(platform.operating_system());
    writer.put_string(platform.architecture());
    writer.put_optional_string(platform.variant());
    writer.put_count(template.modules().len());
    for module in template.modules() {
        writer.put_string(&module.to_string());
    }
    writer.put_presence(template.command().is_some());
    if let Some(command) = template.command() {
        put_command(&mut writer, command);
    }
    let resources = template.resources();
    writer.put_u32(resources.vcpus);
    writer.put_u64(resources.memory_mib);
    writer.put_u64(resources.writable_storage_mib);
    let network = template.network();
    writer.put_u8(network.egress.code());
    writer.put_strings(&network.allow_domains);
    writer.put_strings(&network.allow_cidrs);
    writer.put_u8(network.ingress.code());
    let lifecycle = template.lifecycle();
    writer.put_u64(lifecycle.idle_timeout_seconds);
    writer.put_u64(lifecycle.maximum_lifetime_seconds);
    writer.put_u8(lifecycle.on_idle.code());
    writer.put_count(template.environment().len());
    for entry in template.environment() {
        writer.put_string(&entry.name);
        writer.put_optional_string(entry.value.as_deref());
    }
    writer.put_count(template.secrets().len());
    for secret in template.secrets() {
        writer.put_string(&secret.name);
        writer.put_string(&secret.source);
        writer.put_u8(secret.delivery.code());
        writer.put_optional_string(secret.scope.as_deref());
        writer.put_presence(secret.mode.is_some());
        if let Some(mode) = secret.mode {
            writer.put_u32(mode);
        }
    }
    Sha256::digest(writer.finish()).into()
}
