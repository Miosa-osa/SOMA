//! Secret literals are rejected wherever a Template or module literal is bound, not only in
//! environment values.

mod support;

use soma_template::{EnvironmentName, ModuleKind, RejectionClass};
use support::{EXAMPLE, assert_names, edit, minimal, module, registry, reject, resolve_in};

const LITERAL: &str = "ghp_0123456789abcdef";

fn rejected(text: &str, field: &str) {
    let rejection = reject(text);
    assert_names(&rejection, RejectionClass::SecretLiteral, field, None);
    assert!(
        !rejection.to_string().contains(LITERAL),
        "a rejection must not echo the literal"
    );
}

#[test]
fn command_fields_may_not_carry_a_secret_literal() {
    rejected(
        &edit(
            EXAMPLE,
            "args = []",
            "args = [\"--api-key\", \"sk-ant-api03-xyz\"]",
        ),
        "command.args[1]",
    );
    rejected(
        &edit(
            EXAMPLE,
            "args = []",
            &format!("args = [\"--token={LITERAL}\"]"),
        ),
        "command.args[0]",
    );
    rejected(
        &edit(
            EXAMPLE,
            "program = \"claude\"",
            "program = \"AKIA0123456789ABCDEF\"",
        ),
        "command.program",
    );
    rejected(
        &edit(
            EXAMPLE,
            "working_directory = \"/workspace\"",
            &format!("working_directory = \"/run/{LITERAL}\""),
        ),
        "command.working_directory",
    );
    rejected(
        &edit(
            EXAMPLE,
            "working_directory = \"/workspace\"",
            "working_directory = \"/workspace\"\nuser = \"sk-agent\"",
        ),
        "command.user",
    );
    let benign = edit(
        EXAMPLE,
        "args = []",
        "args = [\"--model\", \"claude-3\", \"--dir=/workspace/a:b\"]",
    );
    assert!(support::resolve_text(&benign).is_ok());
}

#[test]
fn description_may_not_carry_a_secret_literal() {
    rejected(
        &edit(
            EXAMPLE,
            "name = \"claude-code-python\"",
            &format!("name = \"claude-code-python\"\ndescription = \"token: {LITERAL}\""),
        ),
        "description",
    );
    let prose = edit(
        EXAMPLE,
        "name = \"claude-code-python\"",
        "name = \"claude-code-python\"\ndescription = \"Claude Code on Python 3.12\"",
    );
    assert!(support::resolve_text(&prose).is_ok());
}

#[test]
fn secret_sources_and_scopes_may_not_carry_a_secret_literal() {
    rejected(
        &edit(
            EXAMPLE,
            "secret://anthropic/default",
            "secret://anthropic/sk-ant-api03-xyz",
        ),
        "secrets[0].source",
    );
    rejected(
        &edit(
            EXAMPLE,
            "delivery = \"environment\"",
            "delivery = \"file\"\nscope = \"/run/sk-ant-api03-xyz\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
        ),
        "secrets[0].scope",
    );
    rejected(
        &edit(
            EXAMPLE,
            "delivery = \"environment\"",
            &format!("delivery = \"environment\"\nscope = \"{LITERAL}\""),
        ),
        "secrets[0].scope",
    );
}

#[test]
fn module_sealed_values_may_not_carry_a_secret_literal() {
    let leaky = module(ModuleKind::Tools, "leaky", 1)
        .sealed_environment(EnvironmentName::parse("MODE").expect("name"), LITERAL)
        .build()
        .expect("spec");
    let named = module(ModuleKind::Tools, "named", 1)
        .sealed_environment(
            EnvironmentName::parse("GITHUB_TOKEN").expect("name"),
            "plain-looking",
        )
        .build()
        .expect("spec");
    let registry = registry(vec![leaky, named]);
    for (name, field_name) in [("leaky", "MODE"), ("named", "GITHUB_TOKEN")] {
        let rejection = support::rejection(resolve_in(
            &registry,
            &minimal(&[&format!("soma://tools/{name}@1")], ""),
        ));
        assert_names(
            &rejection,
            RejectionClass::SecretLiteral,
            "sealed_environment[0]",
            Some(&format!("soma://tools/{name}@1")),
        );
        assert!(rejection.to_string().contains(field_name));
        assert!(!rejection.to_string().contains(LITERAL));
    }
}
