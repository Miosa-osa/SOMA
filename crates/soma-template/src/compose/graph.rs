//! Deterministic module ordering with cycle and unpinned-input detection.
//!
//! Each Template module is visited in authored order.
//! A module's transitive requirements are placed before it in their declared order, each
//! module appears once, and the first cycle or unpinned reference stops composition with
//! the requiring module and field named.

use std::collections::BTreeSet;

use crate::{
    module::{ModuleIdentity, ModuleRef, ModuleRegistry, ModuleSpec},
    rejection::Rejection,
    schema::Template,
};

pub(super) fn order<'a>(
    template: &Template,
    registry: &'a ModuleRegistry,
) -> Result<Vec<&'a ModuleSpec>, Rejection> {
    let mut walk = Walk {
        registry,
        visiting: Vec::new(),
        done: BTreeSet::new(),
        ordered: Vec::new(),
    };
    let mut listed = BTreeSet::new();
    for (index, reference) in template.modules().iter().enumerate() {
        let field = format!("modules[{index}]");
        let identity = pinned(reference, None, &field)?;
        if !listed.insert(identity.clone()) {
            return Err(Rejection::DuplicateModule {
                field,
                reference: reference.to_string(),
            });
        }
        walk.visit(&identity, None, &field)?;
    }
    Ok(walk.ordered)
}

struct Walk<'a> {
    registry: &'a ModuleRegistry,
    visiting: Vec<ModuleIdentity>,
    done: BTreeSet<ModuleIdentity>,
    ordered: Vec<&'a ModuleSpec>,
}

impl Walk<'_> {
    fn visit(
        &mut self,
        identity: &ModuleIdentity,
        requirer: Option<&ModuleIdentity>,
        field: &str,
    ) -> Result<(), Rejection> {
        if self.done.contains(identity) {
            return Ok(());
        }
        if let Some(start) = self.visiting.iter().position(|member| member == identity) {
            let mut cycle = self.visiting[start..].to_vec();
            cycle.push(identity.clone());
            return Err(Rejection::ModuleCycle {
                module: requirer.cloned().unwrap_or_else(|| identity.clone()),
                field: field.to_owned(),
                cycle,
            });
        }
        let Some(spec) = self.registry.get(identity) else {
            return Err(Rejection::UnknownModule {
                module: requirer.cloned(),
                field: field.to_owned(),
                reference: identity.to_string(),
            });
        };
        self.visiting.push(identity.clone());
        for (index, reference) in spec.requires().iter().enumerate() {
            let field = format!("requires[{index}]");
            let dependency = pinned(reference, Some(identity), &field)?;
            self.visit(&dependency, Some(identity), &field)?;
        }
        self.visiting.pop();
        self.done.insert(identity.clone());
        self.ordered.push(spec);
        Ok(())
    }
}

fn pinned(
    reference: &ModuleRef,
    requirer: Option<&ModuleIdentity>,
    field: &str,
) -> Result<ModuleIdentity, Rejection> {
    reference.pinned().ok_or_else(|| Rejection::UnpinnedInput {
        module: requirer.cloned(),
        field: field.to_owned(),
        reference: reference.to_string(),
    })
}
