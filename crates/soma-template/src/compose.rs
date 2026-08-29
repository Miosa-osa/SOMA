//! Ordered composition of one base workload with its modules.
//!
//! Composition is purely structural: it needs the Template and the module registry and
//! nothing else, which is why the content digest of the composed selection is computed here
//! and can be recomputed from a document without any external input.
//! Every conflict names the module and field responsible instead of choosing a winner.

mod digest;
mod graph;

use std::collections::BTreeMap;

use crate::{
    module::{EnvironmentName, ModuleIdentity, ModuleRegistry, ModuleSpec},
    rejection::Rejection,
    schema::{Command, Template},
};

/// One environment value fixed by a module that neither the Template nor Launch may change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SealedValue {
    pub(crate) module: ModuleIdentity,
    pub(crate) name: EnvironmentName,
    pub(crate) value: String,
}

pub(crate) struct Composition<'a> {
    pub(crate) modules: Vec<&'a ModuleSpec>,
    pub(crate) command: Command,
    pub(crate) sealed: Vec<SealedValue>,
}

impl Composition<'_> {
    /// The SHA-256 digest of the composed selection; see [`digest`].
    pub(crate) fn content_digest(&self, template: &Template) -> [u8; 32] {
        digest::content_digest(template, self)
    }
}

pub(crate) fn compose<'a>(
    template: &Template,
    registry: &'a ModuleRegistry,
) -> Result<Composition<'a>, Rejection> {
    let modules = graph::order(template, registry)?;
    exclusive_fields(&modules)?;
    owned_paths(&modules)?;
    let sealed = sealed_environment(&modules)?;
    template_overrides(template, &sealed)?;
    let command = default_command(template, &modules)?;
    Ok(Composition {
        modules,
        command,
        sealed,
    })
}

fn exclusive_fields(modules: &[&ModuleSpec]) -> Result<(), Rejection> {
    let mut owners: BTreeMap<&str, &ModuleIdentity> = BTreeMap::new();
    for module in modules {
        for (index, field) in module.exclusive_fields().iter().enumerate() {
            if let Some(first) = owners.insert(field, module.identity()) {
                return Err(Rejection::DuplicateExclusiveOwnership {
                    first: first.clone(),
                    second: module.identity().clone(),
                    field: format!("exclusive_fields[{index}]"),
                    owned: field.clone(),
                });
            }
        }
    }
    Ok(())
}

fn owned_paths(modules: &[&ModuleSpec]) -> Result<(), Rejection> {
    let mut owned: Vec<(&crate::module::GuestPath, &ModuleIdentity)> = Vec::new();
    for module in modules {
        for (index, path) in module.owned_paths().iter().enumerate() {
            if let Some((_, first)) = owned
                .iter()
                .find(|(existing, _)| existing.contains(path) || path.contains(existing))
            {
                return Err(Rejection::DuplicateExclusiveOwnership {
                    first: (*first).clone(),
                    second: module.identity().clone(),
                    field: format!("owned_paths[{index}]"),
                    owned: path.as_str().to_owned(),
                });
            }
            owned.push((path, module.identity()));
        }
    }
    Ok(())
}

fn sealed_environment(modules: &[&ModuleSpec]) -> Result<Vec<SealedValue>, Rejection> {
    let mut sealed: Vec<SealedValue> = Vec::new();
    for module in modules {
        for (index, (name, value)) in module.sealed_environment().iter().enumerate() {
            if let Some(first) = sealed.iter().find(|existing| existing.name == *name) {
                if first.value == *value {
                    continue;
                }
                return Err(Rejection::ConflictingSealedEnvironment {
                    module: module.identity().clone(),
                    conflicting_module: Some(first.module.clone()),
                    field: format!("sealed_environment[{index}]"),
                    name: name.as_str().to_owned(),
                });
            }
            sealed.push(SealedValue {
                module: module.identity().clone(),
                name: name.clone(),
                value: value.clone(),
            });
        }
    }
    Ok(sealed)
}

fn template_overrides(template: &Template, sealed: &[SealedValue]) -> Result<(), Rejection> {
    for (index, entry) in template.environment().iter().enumerate() {
        let Some(seal) = sealed.iter().find(|seal| seal.name.as_str() == entry.name) else {
            continue;
        };
        if entry.value.as_deref() == Some(seal.value.as_str()) {
            continue;
        }
        return Err(Rejection::ConflictingSealedEnvironment {
            module: seal.module.clone(),
            conflicting_module: None,
            field: format!("environment[{index}].value"),
            name: entry.name.clone(),
        });
    }
    Ok(())
}

fn default_command(template: &Template, modules: &[&ModuleSpec]) -> Result<Command, Rejection> {
    if let Some(command) = template.command() {
        return Ok(command.clone());
    }
    let mut selected: Option<(&ModuleIdentity, &Command)> = None;
    for module in modules {
        let Some(command) = module.default_command() else {
            continue;
        };
        match selected {
            None => selected = Some((module.identity(), command)),
            Some((first, existing)) if existing != command => {
                return Err(Rejection::ConflictingDefaultCommands {
                    first: first.clone(),
                    second: module.identity().clone(),
                    field: "default_command".to_owned(),
                });
            }
            Some(_) => {}
        }
    }
    selected
        .map(|(_, command)| command.clone())
        .ok_or_else(|| Rejection::MissingDefaultCommand {
            field: "command".to_owned(),
        })
}
