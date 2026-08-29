//! Structural rejection classes: exclusive-ownership conflicts and module-graph failures.

mod support;

use soma_template::{GuestPath, ModuleKind, ModuleRef, Rejection, RejectionClass};
use support::{EXAMPLE, assert_names, edit, minimal, module, registry, reject, resolve_in};

fn path(value: &str) -> GuestPath {
    GuestPath::parse(value).expect("valid guest path")
}

fn reference(value: &str) -> ModuleRef {
    ModuleRef::parse(value).expect("valid reference")
}

#[test]
fn duplicate_exclusive_path_ownership_names_both_modules() {
    let alpha = module(ModuleKind::Tools, "alpha", 1)
        .owned_path(path("/opt/tool"))
        .build()
        .expect("spec");
    let beta = module(ModuleKind::Tools, "beta", 1)
        .owned_path(path("/opt/tool/bin"))
        .build()
        .expect("spec");
    let registry = registry(vec![alpha, beta]);
    let text = minimal(&["soma://tools/alpha@1", "soma://tools/beta@1"], "");
    let rejection = support::rejection(resolve_in(&registry, &text));
    assert_names(
        &rejection,
        RejectionClass::ExclusiveConflict,
        "owned_paths[0]",
        Some("soma://tools/beta@1"),
    );
    let text = rejection.to_string();
    assert!(text.contains("soma://tools/alpha@1"));
    assert!(text.contains("/opt/tool/bin"));
    let sibling = module(ModuleKind::Tools, "gamma", 1)
        .owned_path(path("/opt/tool-two"))
        .build()
        .expect("spec");
    let registry = support::registry(vec![
        module(ModuleKind::Tools, "alpha", 1)
            .owned_path(path("/opt/tool"))
            .build()
            .expect("spec"),
        sibling,
    ]);
    let text = minimal(&["soma://tools/alpha@1", "soma://tools/gamma@1"], "");
    assert!(
        resolve_in(&registry, &text).is_ok(),
        "prefix without a separator is not ownership"
    );
}

#[test]
fn duplicate_exclusive_field_ownership_names_the_field() {
    let first = module(ModuleKind::Workspace, "first", 1)
        .exclusive_field("workspace.root")
        .build()
        .expect("spec");
    let second = module(ModuleKind::Workspace, "second", 1)
        .exclusive_field("workspace.root")
        .build()
        .expect("spec");
    let registry = registry(vec![first, second]);
    let text = minimal(
        &["soma://workspace/first@1", "soma://workspace/second@1"],
        "",
    );
    let rejection = support::rejection(resolve_in(&registry, &text));
    assert_names(
        &rejection,
        RejectionClass::ExclusiveConflict,
        "exclusive_fields[0]",
        Some("soma://workspace/second@1"),
    );
    assert!(matches!(
        rejection,
        Rejection::DuplicateExclusiveOwnership { ref owned, .. } if owned == "workspace.root"
    ));
}

#[test]
fn conflicting_default_commands_name_both_agents() {
    let text = edit(
        &edit(
            EXAMPLE,
            "[command]\nprogram = \"claude\"\nargs = []\nworking_directory = \"/workspace\"\n",
            "",
        ),
        "\"soma://tools/git@1\",",
        "\"soma://tools/git@1\",\n  \"soma://agent/osa@1\",",
    );
    let rejection = reject(&text);
    assert_names(
        &rejection,
        RejectionClass::ExclusiveConflict,
        "default_command",
        Some("soma://agent/osa@1"),
    );
    assert!(rejection.to_string().contains("soma://agent/claude-code@1"));
    let explicit = edit(
        EXAMPLE,
        "\"soma://tools/git@1\",",
        "\"soma://tools/git@1\",\n  \"soma://agent/osa@1\",",
    );
    assert_eq!(support::lock(&explicit).command().program(), "claude");
}

#[test]
fn module_cycles_name_the_requiring_module_and_the_cycle() {
    let a = module(ModuleKind::Tools, "a", 1)
        .requires(reference("soma://tools/b@1"))
        .build()
        .expect("spec");
    let b = module(ModuleKind::Tools, "b", 1)
        .requires(reference("soma://tools/a@1"))
        .build()
        .expect("spec");
    let selfish = module(ModuleKind::Tools, "c", 1)
        .requires(reference("soma://tools/c@1"))
        .build()
        .expect("spec");
    let registry = registry(vec![a, b, selfish]);
    let rejection = support::rejection(resolve_in(&registry, &minimal(&["soma://tools/a@1"], "")));
    assert_names(
        &rejection,
        RejectionClass::ModuleGraph,
        "requires[0]",
        Some("soma://tools/b@1"),
    );
    match &rejection {
        Rejection::ModuleCycle { cycle, .. } => {
            let members: Vec<String> = cycle.iter().map(ToString::to_string).collect();
            assert_eq!(
                members,
                ["soma://tools/a@1", "soma://tools/b@1", "soma://tools/a@1"]
            );
        }
        other => panic!("expected a cycle, got {other}"),
    }
    let rejection = support::rejection(resolve_in(&registry, &minimal(&["soma://tools/c@1"], "")));
    assert_names(
        &rejection,
        RejectionClass::ModuleGraph,
        "requires[0]",
        Some("soma://tools/c@1"),
    );
}

#[test]
fn unpinned_inputs_name_the_module_or_the_template_list() {
    let d = module(ModuleKind::Tools, "d", 1)
        .requires(reference("soma://tools/git"))
        .build()
        .expect("spec");
    let registry = registry(vec![d]);
    let transitive = support::rejection(resolve_in(&registry, &minimal(&["soma://tools/d@1"], "")));
    assert_names(
        &transitive,
        RejectionClass::ModuleGraph,
        "requires[0]",
        Some("soma://tools/d@1"),
    );
    assert!(transitive.to_string().contains("soma://tools/git"));
    let direct = reject(&edit(
        EXAMPLE,
        "\"soma://tools/git@1\"",
        "\"soma://tools/git\"",
    ));
    assert_names(&direct, RejectionClass::ModuleGraph, "modules[1]", None);
    assert!(matches!(direct, Rejection::UnpinnedInput { .. }));
}

#[test]
fn unknown_and_duplicate_modules_name_their_position() {
    let unknown = reject(&edit(
        EXAMPLE,
        "\"soma://tools/git@1\"",
        "\"soma://tools/nope@1\"",
    ));
    assert_names(&unknown, RejectionClass::ModuleGraph, "modules[1]", None);
    assert!(matches!(unknown, Rejection::UnknownModule { .. }));
    let e = module(ModuleKind::Tools, "e", 1)
        .requires(reference("soma://tools/nope@7"))
        .build()
        .expect("spec");
    let transitive = support::rejection(resolve_in(
        &registry(vec![e]),
        &minimal(&["soma://tools/e@1"], ""),
    ));
    assert_names(
        &transitive,
        RejectionClass::ModuleGraph,
        "requires[0]",
        Some("soma://tools/e@1"),
    );
    let duplicate = reject(&edit(
        EXAMPLE,
        "\"soma://tools/git@1\"",
        "\"soma://tools/git@1\",\n  \"soma://tools/git@1\"",
    ));
    assert_names(&duplicate, RejectionClass::ModuleGraph, "modules[2]", None);
    assert!(matches!(duplicate, Rejection::DuplicateModule { .. }));
}

#[test]
fn every_rejection_class_is_reachable() {
    let mut seen = std::collections::BTreeSet::new();
    let d = module(ModuleKind::Tools, "d", 1)
        .requires(reference("soma://tools/git"))
        .build()
        .expect("spec");
    let registry = registry(vec![d]);
    seen.insert(
        support::rejection(resolve_in(&registry, &minimal(&["soma://tools/d@1"], ""))).class(),
    );
    let no_command = edit(
        EXAMPLE,
        "[command]\nprogram = \"claude\"\nargs = []\nworking_directory = \"/workspace\"\n",
        "",
    );
    seen.insert(
        reject(&edit(
            &no_command,
            "\"soma://tools/git@1\",",
            "\"soma://tools/git@1\",\n  \"soma://agent/osa@1\",",
        ))
        .class(),
    );
    seen.insert(reject(&edit(EXAMPLE, "python:3.12-slim", "python:9")).class());
    seen.insert(reject(&edit(EXAMPLE, "[[secrets]]\nname = \"ANTHROPIC_API_KEY\"\nsource = \"secret://anthropic/default\"\ndelivery = \"environment\"\n", "")).class());
    seen.insert(
        reject(&edit(
            EXAMPLE,
            "value = \"true\"",
            "value = \"ghp_1234567890\"",
        ))
        .class(),
    );
    seen.insert(
        reject(&edit(
            EXAMPLE,
            "delivery = \"environment\"",
            "delivery = \"file\"",
        ))
        .class(),
    );
    seen.insert(
        reject(&edit(
            EXAMPLE,
            "ingress = \"deny\"",
            "ingress = \"unrestricted\"",
        ))
        .class(),
    );
    seen.insert(reject(&edit(EXAMPLE, "program = \"claude\"", "program = \"nope\"")).class());
    seen.insert(reject(&edit(EXAMPLE, "vcpus = 2", "vcpus = 0")).class());
    seen.insert(
        reject(&edit(
            EXAMPLE,
            "on_idle = \"destroy\"",
            "on_idle = \"checkpoint\"",
        ))
        .class(),
    );
    let all: std::collections::BTreeSet<RejectionClass> = RejectionClass::ALL.into_iter().collect();
    assert_eq!(seen, all);
}
