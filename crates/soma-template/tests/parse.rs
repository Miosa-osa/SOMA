//! Parsing the versioned document: shape, bounds, and unknown-field rejection.

mod support;

use soma_template::{
    BoundError, EgressIntent, IdleAction, IngressIntent, MAX_DOCUMENT_BYTES, ParseError,
    SecretDelivery, parse_template,
};
use support::{EXAMPLE, edit, parse};

fn error(text: &str) -> ParseError {
    parse_template(text.as_bytes()).expect_err("document must be rejected")
}

fn unknown(field: &str) -> ParseError {
    ParseError::UnknownField {
        field: field.to_owned(),
    }
}

#[test]
fn parses_the_specification_example() {
    let template = parse(EXAMPLE);
    assert_eq!(template.name(), "claude-code-python");
    assert_eq!(template.description(), None);
    assert_eq!(template.workload().image().as_str(), "python:3.12-slim");
    assert_eq!(template.workload().platform().architecture(), "amd64");
    let modules: Vec<String> = template.modules().iter().map(ToString::to_string).collect();
    assert_eq!(
        modules,
        ["soma://agent/claude-code@1", "soma://tools/git@1"]
    );
    let command = template.command().expect("command");
    assert_eq!(command.program(), "claude");
    assert!(command.args().is_empty());
    assert_eq!(command.working_directory(), Some("/workspace"));
    assert_eq!(command.user(), None);
    assert_eq!(template.resources().memory_mib, 2048);
    assert_eq!(template.network().egress, EgressIntent::Deny);
    assert_eq!(template.network().allow_domains, ["api.anthropic.com"]);
    assert_eq!(template.network().ingress, IngressIntent::Deny);
    assert_eq!(template.lifecycle().on_idle, IdleAction::Destroy);
    assert_eq!(template.environment()[0].value.as_deref(), Some("true"));
    assert_eq!(template.secrets()[0].delivery, SecretDelivery::Environment);
    assert_eq!(template.secrets()[0].scope, None);
}

#[test]
fn unknown_fields_are_rejected_with_their_full_path() {
    let root = edit(
        EXAMPLE,
        "name = \"claude-code-python\"",
        "name = \"x\"\nnmae = \"typo\"",
    );
    assert_eq!(error(&root), unknown("nmae"));
    let nested = edit(
        EXAMPLE,
        "platform = \"linux/amd64\"",
        "platform = \"linux/amd64\"\ntag = \"latest\"",
    );
    assert_eq!(error(&nested), unknown("workload.tag"));
    let array = edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"environment\"\nvalue = \"leak\"",
    );
    assert_eq!(error(&array), unknown("secrets[0].value"));
    let network = edit(
        EXAMPLE,
        "ingress = \"deny\"",
        "ingress = \"deny\"\nallow_ports = [443]",
    );
    assert_eq!(error(&network), unknown("network.allow_ports"));
}

#[test]
fn unknown_or_missing_schema_is_rejected_before_anything_else() {
    let future = edit(EXAMPLE, "soma.template/v1alpha1", "soma.template/v2");
    let unsupported = ParseError::UnsupportedSchema {
        found: "soma.template/v2".to_owned(),
    };
    assert_eq!(error(&future), unsupported);
    let missing = edit(EXAMPLE, "schema = \"soma.template/v1alpha1\"\n", "");
    assert_eq!(
        error(&missing),
        ParseError::MissingField {
            field: "schema".to_owned()
        }
    );
    let typed = edit(EXAMPLE, "schema = \"soma.template/v1alpha1\"", "schema = 1");
    assert_eq!(
        error(&typed),
        ParseError::WrongType {
            field: "schema".to_owned(),
            expected: "a string"
        }
    );
    let broken_and_future = edit(&future, "vcpus = 2", "vcpus = \"two\"");
    assert_eq!(error(&broken_and_future), unsupported);
}

#[test]
fn wrong_types_and_missing_required_fields_name_the_field() {
    let wrong = |field: &str, expected: &'static str| ParseError::WrongType {
        field: field.to_owned(),
        expected,
    };
    let integer = "a non-negative integer";
    assert_eq!(
        error(&edit(EXAMPLE, "vcpus = 2", "vcpus = \"2\"")),
        wrong("resources.vcpus", integer)
    );
    assert_eq!(
        error(&edit(EXAMPLE, "memory_mib = 2048", "memory_mib = -1")),
        wrong("resources.memory_mib", integer)
    );
    assert_eq!(
        error(&edit(EXAMPLE, "vcpus = 2", "vcpus = 4294967296")),
        wrong("resources.vcpus", "an integer between 0 and 4294967295")
    );
    let no_workload = edit(
        EXAMPLE,
        "[workload]\nimage = \"python:3.12-slim\"\nplatform = \"linux/amd64\"\n",
        "",
    );
    assert_eq!(
        error(&no_workload),
        ParseError::MissingField {
            field: "workload".to_owned()
        }
    );
    assert_eq!(
        error(&edit(
            EXAMPLE,
            "on_idle = \"destroy\"",
            "on_idle = \"hibernate\""
        )),
        ParseError::InvalidValue {
            field: "lifecycle.on_idle".to_owned(),
            reason: "expected destroy, stop, or checkpoint".to_owned()
        }
    );
    let scheme = edit(
        EXAMPLE,
        "\"soma://agent/claude-code@1\"",
        "\"npm://agent/claude-code@1\"",
    );
    assert_eq!(
        error(&scheme),
        ParseError::InvalidValue {
            field: "modules[0]".to_owned(),
            reason: "module reference must start with soma://".to_owned()
        }
    );
    let platform = error(&edit(
        EXAMPLE,
        "platform = \"linux/amd64\"",
        "platform = \"linux\"",
    ));
    assert_eq!(platform.field(), Some("workload.platform"));
}

#[test]
fn environment_entries_declare_exactly_one_of_value_or_required() {
    let both = edit(
        EXAMPLE,
        "value = \"true\"",
        "value = \"true\"\nrequired = true",
    );
    assert_eq!(error(&both).field(), Some("environment[0].value"));
    let neither = edit(EXAMPLE, "value = \"true\"", "required = false");
    assert_eq!(error(&neither).field(), Some("environment[0].value"));
    let required = edit(EXAMPLE, "value = \"true\"", "required = true");
    assert_eq!(parse(&required).environment()[0].value, None);
}

#[test]
fn optional_tables_default_closed() {
    let text = EXAMPLE
        .replace(
            "[network]\negress = \"deny\"\nallow_domains = [\"api.anthropic.com\"]\ningress = \"deny\"\n",
            "",
        )
        .replace(
            "[command]\nprogram = \"claude\"\nargs = []\nworking_directory = \"/workspace\"\n",
            "",
        );
    let template = parse(&text);
    assert_eq!(template.network().egress, EgressIntent::Deny);
    assert_eq!(template.network().ingress, IngressIntent::Deny);
    assert!(template.network().allow_domains.is_empty());
    assert!(template.command().is_none());
}

#[test]
fn bounds_are_enforced_before_semantics() {
    let oversized = vec![b' '; MAX_DOCUMENT_BYTES + 1];
    assert!(matches!(
        parse_template(&oversized),
        Err(ParseError::Oversized { .. })
    ));
    assert_eq!(
        parse_template(&[0xff, 0xfe, b'a']),
        Err(ParseError::NotUtf8)
    );
    let list: Vec<String> = (0..65)
        .map(|i| format!("\"soma://tools/t{i}@1\""))
        .collect();
    let many = edit(
        EXAMPLE,
        "\"soma://agent/claude-code@1\",\n  \"soma://tools/git@1\",",
        &list.join(",\n"),
    );
    assert_eq!(
        error(&many),
        ParseError::Bound(BoundError::TooMany {
            field: "modules".to_owned(),
            maximum: 64
        })
    );
    let long = "a".repeat(129);
    assert_eq!(
        error(&edit(EXAMPLE, "claude-code-python", &long)),
        ParseError::Bound(BoundError::TooLong {
            field: "name".to_owned(),
            maximum: 128
        })
    );
    assert_eq!(
        error(&edit(
            EXAMPLE,
            "name = \"claude-code-python\"",
            "name = \"\""
        )),
        ParseError::Bound(BoundError::Empty {
            field: "name".to_owned()
        })
    );
    let huge = "x".repeat(4097);
    let value = edit(EXAMPLE, "value = \"true\"", &format!("value = \"{huge}\""));
    assert_eq!(error(&value).field(), Some("environment[0].value"));
}
