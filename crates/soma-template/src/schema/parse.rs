//! `parse_template`: bytes to a shape-checked `soma.template/v1alpha1` document.
//!
//! Parsing checks bounds and shapes only.
//! Semantic rules such as nonzero resources, absolute working directories, and network
//! ceilings are validation rejections that name the responsible field.

use soma::{OciImage, OciPlatform};

use super::{
    Command, EgressIntent, EnvironmentEntry, IdleAction, IngressIntent, Lifecycle, MAX_ARGUMENTS,
    MAX_CIDRS, MAX_DOCUMENT_BYTES, MAX_DOMAINS, MAX_ENVIRONMENT, MAX_MODULES, MAX_NAME_BYTES,
    MAX_SECRETS, Network, Resources, SCHEMA, SecretDelivery, SecretReference, Template, Workload,
    reader::TableReader,
};
use crate::{
    error::{BoundError, ParseError},
    module::ModuleRef,
};

/// Parses one Template document.
///
/// # Errors
///
/// Returns [`ParseError`] for an oversized or non-UTF-8 document, TOML syntax failure,
/// an unsupported `schema`, a missing, unknown, or mistyped field, or a bound violation.
pub fn parse_template(bytes: &[u8]) -> Result<Template, ParseError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ParseError::Oversized {
            length: bytes.len(),
            maximum: MAX_DOCUMENT_BYTES,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::NotUtf8)?;
    let table: toml::Table = text
        .parse()
        .map_err(|error: toml::de::Error| ParseError::Syntax(error.message().to_owned()))?;
    let mut root = TableReader::new(&table, "");
    let schema = root.string("schema")?;
    if schema != SCHEMA {
        return Err(ParseError::UnsupportedSchema {
            found: schema.to_owned(),
        });
    }
    let name = root.string("name")?;
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(BoundError::TooLong {
            field: "name".to_owned(),
            maximum: MAX_NAME_BYTES,
        }
        .into());
    }
    if name.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(BoundError::ForbiddenCharacter {
            field: "name".to_owned(),
        }
        .into());
    }
    let description = root.optional_string("description")?.map(str::to_owned);
    let workload = workload(root.table("workload")?)?;
    let modules = modules(&mut root)?;
    let command = match root.optional_table("command")? {
        Some(reader) => Some(command(reader)?),
        None => None,
    };
    let resources = resources(root.table("resources")?)?;
    let network = match root.optional_table("network")? {
        Some(reader) => network(reader)?,
        None => Network::default(),
    };
    let lifecycle = lifecycle(root.table("lifecycle")?)?;
    let environment = environment(&mut root)?;
    let secrets = secrets(&mut root)?;
    root.finish()?;
    Ok(Template {
        name: name.to_owned(),
        description,
        workload,
        modules,
        command,
        resources,
        network,
        lifecycle,
        environment,
        secrets,
    })
}

fn workload(mut reader: TableReader<'_>) -> Result<Workload, ParseError> {
    let image_field = reader.field("image");
    let image = OciImage::parse(reader.string("image")?).map_err(|_| ParseError::InvalidValue {
        field: image_field,
        reason: "not a bounded OCI image reference".to_owned(),
    })?;
    let platform_field = reader.field("platform");
    let platform =
        platform(reader.string("platform")?).ok_or_else(|| ParseError::InvalidValue {
            field: platform_field,
            reason: "expected <os>/<architecture> or <os>/<architecture>/<variant>".to_owned(),
        })?;
    reader.finish()?;
    Ok(Workload { image, platform })
}

fn platform(text: &str) -> Option<OciPlatform> {
    let mut parts = text.split('/');
    let operating_system = parts.next()?;
    let architecture = parts.next()?;
    let variant = parts.next().map(str::to_owned);
    if parts.next().is_some() {
        return None;
    }
    OciPlatform::new(operating_system, architecture, variant).ok()
}

fn modules(root: &mut TableReader<'_>) -> Result<Vec<ModuleRef>, ParseError> {
    let field = root.field("modules");
    let mut modules = Vec::new();
    for (index, text) in root.strings("modules", MAX_MODULES)?.iter().enumerate() {
        let reference = ModuleRef::parse(text).map_err(|error| ParseError::InvalidValue {
            field: format!("{field}[{index}]"),
            reason: error.to_string(),
        })?;
        modules.push(reference);
    }
    Ok(modules)
}

fn command(mut reader: TableReader<'_>) -> Result<Command, ParseError> {
    let program = reader.string("program")?;
    let args = reader.strings("args", MAX_ARGUMENTS)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let working_directory = reader.optional_string("working_directory")?;
    let user = reader.optional_string("user")?;
    reader.finish()?;
    let mut command = Command::new(program, &args)?;
    if let Some(directory) = working_directory {
        command = command.with_working_directory(directory)?;
    }
    if let Some(user) = user {
        command = command.with_user(user)?;
    }
    Ok(command)
}

fn resources(mut reader: TableReader<'_>) -> Result<Resources, ParseError> {
    let resources = Resources {
        vcpus: reader.u32("vcpus")?,
        memory_mib: reader.u64("memory_mib")?,
        writable_storage_mib: reader.u64("writable_storage_mib")?,
    };
    reader.finish()?;
    Ok(resources)
}

fn network(mut reader: TableReader<'_>) -> Result<Network, ParseError> {
    let egress = match reader.optional_string("egress")? {
        None | Some("deny") => EgressIntent::Deny,
        Some("allowlist") => EgressIntent::Allowlist,
        Some("unrestricted") => EgressIntent::Unrestricted,
        Some(_) => {
            return Err(choice(
                &reader,
                "egress",
                "deny, allowlist, or unrestricted",
            ));
        }
    };
    let allow_domains = reader.strings("allow_domains", MAX_DOMAINS)?;
    let allow_cidrs = reader.strings("allow_cidrs", MAX_CIDRS)?;
    let ingress = match reader.optional_string("ingress")? {
        None | Some("deny") => IngressIntent::Deny,
        Some("unrestricted") => IngressIntent::Unrestricted,
        Some(_) => return Err(choice(&reader, "ingress", "deny or unrestricted")),
    };
    reader.finish()?;
    Ok(Network {
        egress,
        allow_domains,
        allow_cidrs,
        ingress,
    })
}

fn lifecycle(mut reader: TableReader<'_>) -> Result<Lifecycle, ParseError> {
    let idle_timeout_seconds = reader.u64("idle_timeout_seconds")?;
    let maximum_lifetime_seconds = reader.u64("maximum_lifetime_seconds")?;
    let on_idle = match reader.string("on_idle")? {
        "destroy" => IdleAction::Destroy,
        "stop" => IdleAction::Stop,
        "checkpoint" => IdleAction::Checkpoint,
        _ => return Err(choice(&reader, "on_idle", "destroy, stop, or checkpoint")),
    };
    reader.finish()?;
    Ok(Lifecycle {
        idle_timeout_seconds,
        maximum_lifetime_seconds,
        on_idle,
    })
}

fn environment(root: &mut TableReader<'_>) -> Result<Vec<EnvironmentEntry>, ParseError> {
    let mut entries = Vec::new();
    for mut reader in root.tables("environment", MAX_ENVIRONMENT)? {
        let name = reader.string("name")?.to_owned();
        let value = reader.optional_string("value")?.map(str::to_owned);
        let required = reader.optional_bool("required")?.unwrap_or(false);
        let value_field = reader.field("value");
        reader.finish()?;
        if value.is_some() == required {
            return Err(ParseError::InvalidValue {
                field: value_field,
                reason: "declare exactly one of `value` or `required = true`".to_owned(),
            });
        }
        entries.push(EnvironmentEntry { name, value });
    }
    Ok(entries)
}

fn secrets(root: &mut TableReader<'_>) -> Result<Vec<SecretReference>, ParseError> {
    let mut references = Vec::new();
    for mut reader in root.tables("secrets", MAX_SECRETS)? {
        let name = reader.string("name")?.to_owned();
        let source = reader.string("source")?.to_owned();
        let delivery = match reader.string("delivery")? {
            "environment" => SecretDelivery::Environment,
            "file" => SecretDelivery::File,
            "egress-proxy" => SecretDelivery::EgressProxy,
            _ => {
                return Err(choice(
                    &reader,
                    "delivery",
                    "environment, file, or egress-proxy",
                ));
            }
        };
        let scope = reader.optional_string("scope")?.map(str::to_owned);
        let mode = reader.optional_u32("mode")?;
        reader.finish()?;
        references.push(SecretReference {
            name,
            source,
            delivery,
            scope,
            mode,
        });
    }
    Ok(references)
}

fn choice(reader: &TableReader<'_>, key: &str, expected: &str) -> ParseError {
    ParseError::InvalidValue {
        field: reader.field(key),
        reason: format!("expected {expected}"),
    }
}
