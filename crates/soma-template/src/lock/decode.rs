//! Hostile lock decoder; every read is bounded, trailing bytes are rejected, and every
//! decoded record then passes the shape rules in [`super::verify`].

use super::{
    LOCK_MAGIC, LOCK_SCHEMA_VERSION, LockedCommand, LockedEnvironment, LockedModule, LockedSecret,
    MAX_LOCK_ENVIRONMENT, MAX_LOCK_MODULES, TemplateLock, verify,
};
use crate::{
    error::LockError,
    module::{MAX_MODULE_NAME_BYTES, ModuleIdentity, ModuleKind},
    resolve::ResolvedImage,
    schema::{
        IdleAction, Lifecycle, MAX_ARGUMENTS, MAX_SECRETS, MAX_STRING_BYTES, Resources, SCHEMA,
        SecretDelivery,
    },
    validate::{BackendCapabilities, NetworkEnvelope, PolicyCeiling},
    wire::Reader,
};

pub(super) fn decode(bytes: &[u8]) -> Result<TemplateLock, LockError> {
    let mut reader = Reader::new(bytes);
    if reader.take(LOCK_MAGIC.len())? != LOCK_MAGIC {
        return Err(LockError::BadMagic);
    }
    let lock_schema = reader.u16()?;
    if lock_schema != LOCK_SCHEMA_VERSION {
        return Err(LockError::UnsupportedLockSchema(lock_schema));
    }
    let template_schema = reader.string(MAX_STRING_BYTES)?;
    if template_schema != SCHEMA {
        return Err(LockError::UnsupportedTemplateSchema(template_schema));
    }
    let policy_version = reader.u16()?;
    let content_digest = reader.array::<32>()?;
    let image = ResolvedImage::decode(&mut reader)?;
    let modules = modules(&mut reader)?;
    let command = command(&mut reader)?;
    let resources = Resources {
        vcpus: reader.u32()?,
        memory_mib: reader.u64()?,
        writable_storage_mib: reader.u64()?,
    };
    let network = NetworkEnvelope::decode(&mut reader)?;
    let lifecycle = lifecycle(&mut reader)?;
    let environment = environment(&mut reader)?;
    let secrets = secrets(&mut reader)?;
    let ceiling = PolicyCeiling::decode(&mut reader)?;
    let backend = BackendCapabilities::decode(&mut reader)?;
    reader.finish()?;
    verify::image(&image, &backend)?;
    verify::command(&command)?;
    verify::resources(&resources, &backend)?;
    verify::lifecycle(&lifecycle, &backend)?;
    verify::environment(&environment)?;
    verify::secrets(&secrets, &network)?;
    Ok(TemplateLock {
        template_schema,
        policy_version,
        content_digest,
        image,
        modules,
        command,
        resources,
        network,
        lifecycle,
        environment,
        secrets,
        ceiling,
        backend,
    })
}

fn identity(reader: &mut Reader<'_>, field: &'static str) -> Result<ModuleIdentity, LockError> {
    let code = reader.u8()?;
    let kind =
        ModuleKind::from_code(code).ok_or(LockError::InvalidDiscriminant { field, value: code })?;
    let name = reader.string(MAX_MODULE_NAME_BYTES)?;
    let version = reader.u32()?;
    ModuleIdentity::new(kind, &name, version).map_err(|_| LockError::InvalidField { field })
}

fn modules(reader: &mut Reader<'_>) -> Result<Vec<LockedModule>, LockError> {
    let count = reader.count(MAX_LOCK_MODULES)?;
    let mut modules = Vec::with_capacity(count);
    for _ in 0..count {
        let module = LockedModule {
            identity: identity(reader, "modules")?,
            schema_version: reader.u16()?,
            digest: reader.array()?,
        };
        if module.schema_version == 0
            || modules
                .iter()
                .any(|existing: &LockedModule| existing.identity == module.identity)
        {
            return Err(LockError::InvalidField { field: "modules" });
        }
        modules.push(module);
    }
    Ok(modules)
}

fn command(reader: &mut Reader<'_>) -> Result<LockedCommand, LockError> {
    Ok(LockedCommand {
        program: reader.string(MAX_STRING_BYTES)?,
        args: reader.strings(MAX_ARGUMENTS, MAX_STRING_BYTES)?,
        working_directory: reader.string(MAX_STRING_BYTES)?,
        user: reader.string(MAX_STRING_BYTES)?,
    })
}

fn lifecycle(reader: &mut Reader<'_>) -> Result<Lifecycle, LockError> {
    let idle_timeout_seconds = reader.u64()?;
    let maximum_lifetime_seconds = reader.u64()?;
    let code = reader.u8()?;
    let on_idle = IdleAction::from_code(code).ok_or(LockError::InvalidDiscriminant {
        field: "lifecycle.on_idle",
        value: code,
    })?;
    Ok(Lifecycle {
        idle_timeout_seconds,
        maximum_lifetime_seconds,
        on_idle,
    })
}

fn environment(reader: &mut Reader<'_>) -> Result<Vec<LockedEnvironment>, LockError> {
    let count = reader.count(MAX_LOCK_ENVIRONMENT)?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let name = reader.string(MAX_STRING_BYTES)?;
        let value = reader.optional_string(MAX_STRING_BYTES)?;
        let sealed_by = if reader.presence()? {
            Some(identity(reader, "environment.sealed_by")?)
        } else {
            None
        };
        entries.push(LockedEnvironment {
            name,
            value,
            sealed_by,
        });
    }
    Ok(entries)
}

fn secrets(reader: &mut Reader<'_>) -> Result<Vec<LockedSecret>, LockError> {
    let count = reader.count(MAX_SECRETS)?;
    let mut secrets = Vec::with_capacity(count);
    for _ in 0..count {
        let name = reader.string(MAX_STRING_BYTES)?;
        let source = reader.string(MAX_STRING_BYTES)?;
        let code = reader.u8()?;
        let delivery = SecretDelivery::from_code(code).ok_or(LockError::InvalidDiscriminant {
            field: "secrets.delivery",
            value: code,
        })?;
        let scope = reader.string(MAX_STRING_BYTES)?;
        let mode = if reader.presence()? {
            Some(reader.u32()?)
        } else {
            None
        };
        secrets.push(LockedSecret {
            name,
            source,
            delivery,
            scope,
            mode,
        });
    }
    Ok(secrets)
}
