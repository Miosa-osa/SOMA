//! Platform, resource, lifecycle, description, command, module-value, and executable checks.

use super::{FilesystemOracle, backend::BackendCapabilities, secret, syntax};
use crate::{
    compose::Composition,
    error::{ExternalDependency, TemplateError},
    lock::LockedCommand,
    module::{HealthProbe, ModuleSpec},
    rejection::{InvalidReason, Rejection},
    resolve::ResolvedImage,
    schema::{Command, DEFAULT_USER, DEFAULT_WORKING_DIRECTORY, Template},
};

pub(super) fn invalid(field: String, reason: InvalidReason) -> Rejection {
    Rejection::InvalidValue {
        module: None,
        field,
        reason,
    }
}

pub(super) fn platforms(
    template: &Template,
    composition: &Composition<'_>,
    backend: &BackendCapabilities,
) -> Result<(), Rejection> {
    let platform = template.workload().platform();
    let text = super::policy::platform_key(platform);
    if !backend.supports_platform(platform) {
        return Err(Rejection::UnsupportedPlatform {
            module: None,
            field: "workload.platform".to_owned(),
            platform: text,
        });
    }
    for module in &composition.modules {
        if !module.platforms().contains(platform) {
            return Err(Rejection::UnsupportedPlatform {
                module: Some(module.identity().clone()),
                field: "platforms".to_owned(),
                platform: text,
            });
        }
    }
    Ok(())
}

pub(super) fn resources(
    template: &Template,
    backend: &BackendCapabilities,
) -> Result<(), Rejection> {
    let resources = template.resources();
    let limits = backend.limits();
    dimension(
        "resources.vcpus",
        u64::from(resources.vcpus),
        u64::from(limits.max_vcpus.min(u32::from(u16::MAX))),
    )?;
    dimension(
        "resources.memory_mib",
        resources.memory_mib,
        limits.max_memory_mib,
    )?;
    // Zero writable storage is a sandbox with no writable disk at all, which is a machine the
    // backend can build, so it is accepted where zero vCPUs or zero memory are not.
    bounded(
        "resources.writable_storage_mib",
        resources.writable_storage_mib,
        limits.max_writable_storage_mib,
    )
}

fn dimension(field: &str, value: u64, maximum: u64) -> Result<(), Rejection> {
    if value == 0 {
        return Err(invalid(field.to_owned(), InvalidReason::Zero));
    }
    bounded(field, value, maximum)
}

fn bounded(field: &str, value: u64, maximum: u64) -> Result<(), Rejection> {
    if value > maximum {
        return Err(invalid(
            field.to_owned(),
            InvalidReason::ExceedsMaximum { maximum },
        ));
    }
    Ok(())
}

pub(super) fn lifecycle(
    template: &Template,
    backend: &BackendCapabilities,
) -> Result<(), Rejection> {
    let lifecycle = template.lifecycle();
    syntax::timeout(lifecycle.idle_timeout_seconds)
        .map_err(|reason| invalid("lifecycle.idle_timeout_seconds".to_owned(), reason))?;
    syntax::timeout(lifecycle.maximum_lifetime_seconds)
        .map_err(|reason| invalid("lifecycle.maximum_lifetime_seconds".to_owned(), reason))?;
    if lifecycle.idle_timeout_seconds > lifecycle.maximum_lifetime_seconds {
        return Err(invalid(
            "lifecycle.idle_timeout_seconds".to_owned(),
            InvalidReason::TimeoutOrdering,
        ));
    }
    if !backend.supports_idle_action(lifecycle.on_idle) {
        return Err(Rejection::UnsupportedLifecycleAction {
            field: "lifecycle.on_idle".to_owned(),
            action: lifecycle.on_idle.as_str().to_owned(),
        });
    }
    Ok(())
}

/// A non-content field is still a committed file, so a credential in it is rejected.
pub(super) fn description(template: &Template) -> Result<(), Rejection> {
    match template.description() {
        Some(text) if secret::embedded_secret(text) => Err(literal("description", "description")),
        _ => Ok(()),
    }
}

pub(super) fn command(command: &Command) -> Result<LockedCommand, Rejection> {
    if command
        .program()
        .bytes()
        .any(|byte| byte.is_ascii_control())
    {
        return Err(invalid(
            "command.program".to_owned(),
            InvalidReason::ForbiddenCharacter,
        ));
    }
    let working_directory = command
        .working_directory()
        .unwrap_or(DEFAULT_WORKING_DIRECTORY);
    let working_directory = syntax::absolute_path(working_directory)
        .map_err(|reason| invalid("command.working_directory".to_owned(), reason))?;
    let user = command.user().unwrap_or(DEFAULT_USER);
    syntax::user(user).map_err(|reason| invalid("command.user".to_owned(), reason))?;
    if secret::embedded_secret(command.program()) {
        return Err(literal("command.program", "program"));
    }
    for (index, argument) in command.args().iter().enumerate() {
        if secret::embedded_secret(argument) {
            return Err(literal(&format!("command.args[{index}]"), "args"));
        }
    }
    if secret::embedded_secret(working_directory.as_str()) {
        return Err(literal("command.working_directory", "working_directory"));
    }
    if secret::embedded_secret(user) {
        return Err(literal("command.user", "user"));
    }
    Ok(LockedCommand {
        program: command.program().to_owned(),
        args: command.args().to_vec(),
        working_directory: working_directory.as_str().to_owned(),
        user: user.to_owned(),
    })
}

fn literal(field: &str, name: &str) -> Rejection {
    Rejection::SecretLiteral {
        module: None,
        field: field.to_owned(),
        name: name.to_owned(),
    }
}

pub(super) fn modules(composition: &Composition<'_>) -> Result<(), Rejection> {
    for module in &composition.modules {
        let module_invalid = |field: String, reason| Rejection::InvalidValue {
            module: Some(module.identity().clone()),
            field,
            reason,
        };
        for (index, destination) in module.destinations().iter().enumerate() {
            syntax::domain(destination.host())
                .and_then(|()| syntax::port(destination.port()))
                .map_err(|reason| module_invalid(format!("destinations[{index}]"), reason))?;
        }
        match module.health_probe() {
            Some(HealthProbe::Tcp { port }) => syntax::port(*port)
                .map_err(|reason| module_invalid("health_probe.port".to_owned(), reason))?,
            Some(HealthProbe::Command {
                timeout_seconds, ..
            }) => syntax::timeout(u64::from(*timeout_seconds)).map_err(|reason| {
                module_invalid("health_probe.timeout_seconds".to_owned(), reason)
            })?,
            None => {}
        }
        for (index, (name, value)) in module.sealed_environment().iter().enumerate() {
            if secret::environment_literal(&composition.modules, name.as_str(), value) {
                return Err(Rejection::SecretLiteral {
                    module: Some(module.identity().clone()),
                    field: format!("sealed_environment[{index}]"),
                    name: name.as_str().to_owned(),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn executable(
    command: &LockedCommand,
    modules: &[&ModuleSpec],
    image: &ResolvedImage,
    oracle: &dyn FilesystemOracle,
) -> Result<(), TemplateError> {
    let program = command.program.as_str();
    let provided = modules.iter().any(|module| {
        module.executables().iter().any(|path| {
            path.as_str() == program || (!program.contains('/') && path.file_name() == program)
        })
    });
    if provided {
        return Ok(());
    }
    let present =
        oracle
            .executable_present(image, program)
            .map_err(|error| TemplateError::Unavailable {
                dependency: ExternalDependency::FilesystemOracle,
                detail: error.to_string(),
            })?;
    if present {
        return Ok(());
    }
    Err(Rejection::ExecutableAbsent {
        field: "command.program".to_owned(),
        program: program.to_owned(),
    }
    .into())
}
