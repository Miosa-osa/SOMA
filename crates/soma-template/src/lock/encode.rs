//! Canonical lock encoder; see the layout table in the parent module.

use super::{LOCK_MAGIC, LOCK_SCHEMA_VERSION, TemplateLock};
use crate::{module::ModuleIdentity, wire::Writer};

pub(super) fn encode(lock: &TemplateLock) -> Vec<u8> {
    let mut writer = Writer::with_capacity(1024);
    writer.put_bytes(LOCK_MAGIC);
    writer.put_u16(LOCK_SCHEMA_VERSION);
    writer.put_string(&lock.template_schema);
    writer.put_u16(lock.policy_version);
    writer.put_bytes(&lock.content_digest);
    lock.image.encode(&mut writer);
    writer.put_count(lock.modules.len());
    for module in &lock.modules {
        put_identity(&mut writer, &module.identity);
        writer.put_u16(module.schema_version);
        writer.put_bytes(&module.digest);
    }
    writer.put_string(&lock.command.program);
    writer.put_strings(&lock.command.args);
    writer.put_string(&lock.command.working_directory);
    writer.put_string(&lock.command.user);
    writer.put_u32(lock.resources.vcpus);
    writer.put_u64(lock.resources.memory_mib);
    writer.put_u64(lock.resources.writable_storage_mib);
    lock.network.encode(&mut writer);
    writer.put_u64(lock.lifecycle.idle_timeout_seconds);
    writer.put_u64(lock.lifecycle.maximum_lifetime_seconds);
    writer.put_u8(lock.lifecycle.on_idle.code());
    writer.put_count(lock.environment.len());
    for entry in &lock.environment {
        writer.put_string(&entry.name);
        writer.put_optional_string(entry.value.as_deref());
        writer.put_presence(entry.sealed_by.is_some());
        if let Some(module) = &entry.sealed_by {
            put_identity(&mut writer, module);
        }
    }
    writer.put_count(lock.secrets.len());
    for secret in &lock.secrets {
        writer.put_string(&secret.name);
        writer.put_string(&secret.source);
        writer.put_u8(secret.delivery.code());
        writer.put_string(&secret.scope);
        writer.put_presence(secret.mode.is_some());
        if let Some(mode) = secret.mode {
            writer.put_u32(mode);
        }
    }
    lock.ceiling.encode(&mut writer);
    lock.backend.encode(&mut writer);
    writer.finish()
}

pub(super) fn put_identity(writer: &mut Writer, identity: &ModuleIdentity) {
    writer.put_u8(identity.kind().code());
    writer.put_string(identity.name());
    writer.put_u32(identity.version());
}
