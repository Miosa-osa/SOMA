//! Ordered composition: sealed environment, default commands, and transitive ordering.

mod support;

use soma_template::{EnvironmentName, ModuleKind, ModuleRef, Rejection, RejectionClass};
use support::{EXAMPLE, assert_names, edit, lock, minimal, module, registry, reject, resolve_in};

#[test]
fn template_may_not_override_a_sealed_value() {
    let text = edit(
        EXAMPLE,
        "value = \"true\"",
        "value = \"true\"\n\n[[environment]]\nname = \"GIT_TERMINAL_PROMPT\"\nvalue = \"1\"",
    );
    let rejection = reject(&text);
    assert_names(
        &rejection,
        RejectionClass::ExclusiveConflict,
        "environment[1].value",
        Some("soma://tools/git@1"),
    );
    assert!(matches!(
        rejection,
        Rejection::ConflictingSealedEnvironment {
            conflicting_module: None,
            ..
        }
    ));
    let same = edit(
        EXAMPLE,
        "value = \"true\"",
        "value = \"true\"\n\n[[environment]]\nname = \"GIT_TERMINAL_PROMPT\"\nvalue = \"0\"",
    );
    let restated = lock(&same);
    let baseline = support::example();
    assert_eq!(restated.environment().len(), 2);
    let sealed = restated
        .environment()
        .iter()
        .find(|entry| entry.name() == "GIT_TERMINAL_PROMPT")
        .expect("sealed name is locked");
    assert_eq!(sealed.value(), Some("0"));
    assert_eq!(
        sealed.sealed_by().map(ToString::to_string).as_deref(),
        Some("soma://tools/git@1"),
        "restating a seal must not unseal it"
    );
    assert_eq!(restated.environment(), baseline.environment());
    assert_eq!(restated.encode(), baseline.encode());
    assert_eq!(restated.id(), baseline.id());
}

#[test]
fn modules_may_not_seal_one_name_differently() {
    let name = || EnvironmentName::parse("MODE").expect("name");
    let one = module(ModuleKind::Tools, "one", 1)
        .sealed_environment(name(), "a")
        .build()
        .expect("spec");
    let two = module(ModuleKind::Tools, "two", 1)
        .sealed_environment(name(), "b")
        .build()
        .expect("spec");
    let agree = module(ModuleKind::Tools, "agree", 1)
        .sealed_environment(name(), "a")
        .build()
        .expect("spec");
    let registry = registry(vec![one, two, agree]);
    let rejection = support::rejection(resolve_in(
        &registry,
        &minimal(&["soma://tools/one@1", "soma://tools/two@1"], ""),
    ));
    assert_names(
        &rejection,
        RejectionClass::ExclusiveConflict,
        "sealed_environment[0]",
        Some("soma://tools/two@1"),
    );
    assert!(rejection.to_string().contains("soma://tools/one@1"));
    let lock = resolve_in(
        &registry,
        &minimal(&["soma://tools/one@1", "soma://tools/agree@1"], ""),
    )
    .expect("same value");
    let sealed: Vec<(&str, Option<&str>, String)> = lock
        .environment()
        .iter()
        .map(|e| {
            (
                e.name(),
                e.value(),
                e.sealed_by().map(ToString::to_string).unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        sealed,
        [("MODE", Some("a"), "soma://tools/one@1".to_owned())]
    );
}

#[test]
fn module_default_command_fills_an_omitted_command_table() {
    let text = edit(
        EXAMPLE,
        "[command]\nprogram = \"claude\"\nargs = []\nworking_directory = \"/workspace\"\n",
        "",
    );
    let lock = lock(&text);
    assert_eq!(lock.command().program(), "claude");
    assert!(lock.command().args().is_empty());
    assert_eq!(lock.command().working_directory(), "/");
    assert_eq!(lock.command().user(), "root");
}

#[test]
fn no_default_command_anywhere_is_rejected() {
    let text = minimal(&["soma://tools/git@1"], "");
    let text = edit(&text, "[command]\nprogram = \"sh\"\n", "");
    let rejection = reject(&text);
    assert_names(
        &rejection,
        RejectionClass::ExclusiveConflict,
        "command",
        None,
    );
    assert!(matches!(rejection, Rejection::MissingDefaultCommand { .. }));
}

#[test]
fn transitive_requirements_are_ordered_before_their_requirer_once() {
    let base = module(ModuleKind::Tools, "base", 1).build().expect("spec");
    let mid = module(ModuleKind::Tools, "mid", 1)
        .requires(ModuleRef::parse("soma://tools/base@1").expect("reference"))
        .build()
        .expect("spec");
    let top = module(ModuleKind::Tools, "top", 1)
        .requires(ModuleRef::parse("soma://tools/mid@1").expect("reference"))
        .requires(ModuleRef::parse("soma://tools/base@1").expect("reference"))
        .build()
        .expect("spec");
    let registry = registry(vec![base, mid, top]);
    let order = |modules: &[&str]| -> Vec<String> {
        resolve_in(&registry, &minimal(modules, ""))
            .expect("resolves")
            .modules()
            .iter()
            .map(|module| module.identity().to_string())
            .collect()
    };
    assert_eq!(
        order(&["soma://tools/top@1"]),
        [
            "soma://tools/base@1",
            "soma://tools/mid@1",
            "soma://tools/top@1"
        ]
    );
    assert_eq!(
        order(&["soma://tools/top@1", "soma://tools/base@1"]),
        [
            "soma://tools/base@1",
            "soma://tools/mid@1",
            "soma://tools/top@1"
        ]
    );
    assert_eq!(
        order(&["soma://tools/base@1", "soma://tools/top@1"]),
        [
            "soma://tools/base@1",
            "soma://tools/mid@1",
            "soma://tools/top@1"
        ]
    );
    assert_ne!(
        resolve_in(&registry, &minimal(&["soma://tools/top@1"], ""))
            .expect("a")
            .id(),
        resolve_in(
            &registry,
            &minimal(&["soma://tools/top@1", "soma://tools/git@1"], "")
        )
        .expect("b")
        .id()
    );
}

#[test]
fn a_module_without_requirements_composes_alone() {
    let lock = resolve_in(&registry(Vec::new()), &minimal(&[], "")).expect("empty module list");
    assert!(lock.modules().is_empty());
    assert!(lock.environment().is_empty());
    assert_eq!(lock.command().program(), "sh");
}
