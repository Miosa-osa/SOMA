//! Secret delivery targets are exclusive: environment names, guest files, and destinations.

mod support;

use soma_template::{Rejection, RejectionClass};
use support::{EXAMPLE, assert_names, edit, reject};

fn secret(name: &str, delivery: &str, scope: &str) -> String {
    format!(
        "\n[[secrets]]\nname = \"{name}\"\nsource = \"secret://vault/{}\"\ndelivery = \"{delivery}\"\nscope = \"{scope}\"\n",
        name.to_ascii_lowercase()
    )
}

fn conflict(text: &str, field: &str, module: Option<&str>, target: &str) {
    let rejection = reject(text);
    assert_names(&rejection, RejectionClass::ExclusiveConflict, field, module);
    assert!(
        matches!(rejection, Rejection::ConflictingDeliveryTarget { .. }),
        "{rejection}"
    );
    assert!(rejection.to_string().contains(target), "{rejection}");
}

#[test]
fn environment_delivery_targets_are_exclusive() {
    let shared = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "environment", "SHARED"),
        secret("SECOND", "environment", "SHARED")
    );
    conflict(&shared, "secrets[2].scope", None, "SHARED");
    let literal = format!("{EXAMPLE}{}", secret("CI_OVERRIDE", "environment", "CI"));
    conflict(&literal, "secrets[1].scope", None, "CI");
    let sealed = format!(
        "{EXAMPLE}{}",
        secret("PROMPT", "environment", "GIT_TERMINAL_PROMPT")
    );
    conflict(
        &sealed,
        "secrets[1].scope",
        Some("soma://tools/git@1"),
        "GIT_TERMINAL_PROMPT",
    );
    let required = edit(
        EXAMPLE,
        "[[secrets]]",
        "[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true\n\n[[secrets]]",
    );
    conflict(&required, "secrets[0].scope", None, "ANTHROPIC_API_KEY");
    let distinct = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "environment", "ONE"),
        secret("SECOND", "environment", "TWO")
    );
    assert!(support::resolve_text(&distinct).is_ok());
}

#[test]
fn file_delivery_targets_are_exclusive_and_outside_module_owned_paths() {
    let same = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "file", "/run/secrets/key"),
        secret("SECOND", "file", "/run/secrets/key")
    );
    conflict(&same, "secrets[2].scope", None, "/run/secrets/key");
    let nested = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "file", "/run/secrets"),
        secret("SECOND", "file", "/run/secrets/key")
    );
    conflict(&nested, "secrets[2].scope", None, "/run/secrets/key");
    let reversed = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "file", "/run/secrets/key"),
        secret("SECOND", "file", "/run/secrets")
    );
    conflict(&reversed, "secrets[2].scope", None, "/run/secrets");
    let owned = format!(
        "{EXAMPLE}{}",
        secret("BINARY", "file", "/usr/local/bin/claude")
    );
    conflict(
        &owned,
        "secrets[1].scope",
        Some("soma://agent/claude-code@1"),
        "/usr/local/bin/claude",
    );
    let inside = format!(
        "{EXAMPLE}{}",
        secret(
            "INSIDE",
            "file",
            "/usr/local/lib/soma/agents/claude-code/key"
        )
    );
    conflict(
        &inside,
        "secrets[1].scope",
        Some("soma://agent/claude-code@1"),
        "/usr/local/lib/soma/agents/claude-code/key",
    );
    let above = format!("{EXAMPLE}{}", secret("ABOVE", "file", "/usr/lib"));
    conflict(
        &above,
        "secrets[1].scope",
        Some("soma://tools/git@1"),
        "/usr/lib",
    );
    let siblings = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "file", "/run/secrets/one"),
        secret("SECOND", "file", "/run/secrets/two")
    );
    assert!(support::resolve_text(&siblings).is_ok());
}

#[test]
fn egress_proxy_targets_are_exclusive() {
    let same = format!(
        "{EXAMPLE}{}{}",
        secret("FIRST", "egress-proxy", "api.anthropic.com"),
        secret("SECOND", "egress-proxy", "api.anthropic.com")
    );
    conflict(&same, "secrets[2].scope", None, "api.anthropic.com");
    let one = format!(
        "{EXAMPLE}{}",
        secret("FIRST", "egress-proxy", "api.anthropic.com")
    );
    assert!(support::resolve_text(&one).is_ok());
}
