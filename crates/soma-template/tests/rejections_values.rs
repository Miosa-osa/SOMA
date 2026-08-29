//! Rejection classes decided from the document itself: secret literals, absent executables,
//! and invalid values.

mod support;

use soma_template::{
    ExternalDependency, FilesystemOracle, InvalidReason, OracleError, Rejection, RejectionClass,
    ResolvedImage, TemplateError, resolve,
};
use support::{EXAMPLE, assert_names, backend, ceiling, edit, parse, reject, resolver};

fn with_environment(name: &str, value: &str) -> String {
    edit(
        EXAMPLE,
        "[[secrets]]",
        &format!("[[environment]]\nname = \"{name}\"\nvalue = '{value}'\n\n[[secrets]]"),
    )
}

#[test]
fn secret_literals_are_rejected_by_module_contract_name_and_value_shape() {
    let cases = [
        ("ANTHROPIC_API_KEY", "plain-looking"),
        ("GITHUB_TOKEN", "not-a-known-shape"),
        ("MODE", "ghp_0123456789abcdef"),
        ("MODE", "sk-ant-api03-xyz"),
        ("MODE", "AKIA0123456789ABCDEF"),
        ("MODE", "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2ln"),
        ("MODE", "-----BEGIN RSA PRIVATE KEY-----"),
    ];
    for (name, value) in cases {
        let rejection = reject(&with_environment(name, value));
        assert_names(
            &rejection,
            RejectionClass::SecretLiteral,
            "environment[1].value",
            None,
        );
        assert!(rejection.to_string().contains(name));
        assert!(
            !rejection.to_string().contains(value),
            "a rejection must not echo the literal"
        );
    }
    assert!(support::resolve_text(&with_environment("MODE", "AKIAtooshort")).is_ok());
    for source in [
        "sk-ant-literal",
        "secret://",
        "secret://with space",
        "vault/path",
    ] {
        let text = edit(EXAMPLE, "secret://anthropic/default", source);
        assert_names(
            &reject(&text),
            RejectionClass::SecretLiteral,
            "secrets[0].source",
            None,
        );
    }
}

#[test]
fn absent_executable_names_the_command_program() {
    let rejection = reject(&edit(
        EXAMPLE,
        "program = \"claude\"",
        "program = \"python3.13\"",
    ));
    assert_names(
        &rejection,
        RejectionClass::ExecutableAbsent,
        "command.program",
        None,
    );
    assert!(rejection.to_string().contains("python3.13"));
    for program in [
        "python3",
        "/usr/local/bin/python3",
        "sh",
        "claude",
        "/usr/local/bin/claude",
    ] {
        let text = edit(
            EXAMPLE,
            "program = \"claude\"",
            &format!("program = \"{program}\""),
        );
        assert!(support::resolve_text(&text).is_ok(), "{program}");
    }
    let relative = edit(EXAMPLE, "program = \"claude\"", "program = \"bin/claude\"");
    assert_eq!(reject(&relative).class(), RejectionClass::ExecutableAbsent);
}

#[test]
fn oracle_outage_is_not_a_rejection() {
    struct Down;
    impl FilesystemOracle for Down {
        fn executable_present(&self, _: &ResolvedImage, _: &str) -> Result<bool, OracleError> {
            Err(OracleError::new("store unreachable"))
        }
    }
    let text = edit(EXAMPLE, "program = \"claude\"", "program = \"python3\"");
    let error =
        resolve(&parse(&text), &resolver(), &ceiling(), &backend(), &Down).expect_err("outage");
    assert!(matches!(
        error,
        TemplateError::Unavailable {
            dependency: ExternalDependency::FilesystemOracle,
            ..
        }
    ));
}

fn invalid(text: &str, field: &str, reason: InvalidReason) {
    let rejection = reject(text);
    assert_names(&rejection, RejectionClass::InvalidValue, field, None);
    assert!(
        matches!(rejection, Rejection::InvalidValue { reason: found, .. } if found == reason),
        "{rejection}"
    );
}

#[test]
fn invalid_resources_and_lifecycle_name_the_dimension() {
    invalid(
        &edit(EXAMPLE, "vcpus = 2", "vcpus = 0"),
        "resources.vcpus",
        InvalidReason::Zero,
    );
    invalid(
        &edit(EXAMPLE, "memory_mib = 2048", "memory_mib = 32768"),
        "resources.memory_mib",
        InvalidReason::ExceedsMaximum { maximum: 16_384 },
    );
    invalid(
        &edit(
            EXAMPLE,
            "writable_storage_mib = 10240",
            "writable_storage_mib = 0",
        ),
        "resources.writable_storage_mib",
        InvalidReason::Zero,
    );
    invalid(
        &edit(
            EXAMPLE,
            "idle_timeout_seconds = 900",
            "idle_timeout_seconds = 0",
        ),
        "lifecycle.idle_timeout_seconds",
        InvalidReason::InvalidTimeout,
    );
    invalid(
        &edit(
            EXAMPLE,
            "idle_timeout_seconds = 900",
            "idle_timeout_seconds = 20000",
        ),
        "lifecycle.idle_timeout_seconds",
        InvalidReason::TimeoutOrdering,
    );
    invalid(
        &edit(
            EXAMPLE,
            "maximum_lifetime_seconds = 14400",
            "maximum_lifetime_seconds = 99999999999",
        ),
        "lifecycle.maximum_lifetime_seconds",
        InvalidReason::InvalidTimeout,
    );
}

#[test]
fn invalid_command_shape_names_the_command_field() {
    invalid(
        &edit(
            EXAMPLE,
            "working_directory = \"/workspace\"",
            "working_directory = \"workspace\"",
        ),
        "command.working_directory",
        InvalidReason::NotAbsolutePath,
    );
    invalid(
        &edit(
            EXAMPLE,
            "working_directory = \"/workspace\"",
            "working_directory = \"/work/../space\"",
        ),
        "command.working_directory",
        InvalidReason::NotNormalizedPath,
    );
    invalid(
        &edit(
            EXAMPLE,
            "working_directory = \"/workspace\"",
            "working_directory = \"/workspace\"\nuser = \"Root\"",
        ),
        "command.user",
        InvalidReason::InvalidUser,
    );
    let user = edit(
        EXAMPLE,
        "working_directory = \"/workspace\"",
        "working_directory = \"/workspace\"\nuser = \"agent\"",
    );
    assert_eq!(support::lock(&user).command().user(), "agent");
}

#[test]
fn invalid_secret_mode_and_network_values_name_the_entry() {
    invalid(
        &edit(
            EXAMPLE,
            "delivery = \"environment\"",
            "delivery = \"file\"\nscope = \"/run/key\"\nmode = 420",
        ),
        "secrets[0].mode",
        InvalidReason::InvalidMode,
    );
    invalid(
        &edit(
            EXAMPLE,
            "delivery = \"environment\"",
            "delivery = \"environment\"\nmode = 256",
        ),
        "secrets[0].mode",
        InvalidReason::InvalidMode,
    );
    invalid(
        &edit(EXAMPLE, "\"api.anthropic.com\"]", "\"Bad_Domain\"]"),
        "network.allow_domains[0]",
        InvalidReason::InvalidDomain,
    );
    invalid(
        &edit(
            EXAMPLE,
            "ingress = \"deny\"",
            "allow_cidrs = [\"10.0.0.0\"]\ningress = \"deny\"",
        ),
        "network.allow_cidrs[0]",
        InvalidReason::InvalidCidr,
    );
    invalid(
        &edit(
            &edit(EXAMPLE, "allow_domains = [\"api.anthropic.com\"]\n", ""),
            "egress = \"deny\"",
            "egress = \"allowlist\"",
        ),
        "network.egress",
        InvalidReason::EmptyAllowlist,
    );
    invalid(
        &edit(EXAMPLE, "egress = \"deny\"", "egress = \"unrestricted\""),
        "network.allow_domains[0]",
        InvalidReason::ContradictoryEgress,
    );
}

#[test]
fn invalid_environment_names_name_the_entry() {
    invalid(
        &edit(EXAMPLE, "name = \"CI\"", "name = \"1BAD\""),
        "environment[0].name",
        InvalidReason::ForbiddenCharacter,
    );
    invalid(
        &with_environment("CI", "again"),
        "environment[1].name",
        InvalidReason::Duplicate,
    );
}
