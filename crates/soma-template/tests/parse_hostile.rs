//! Garbage, mutated, and deeply nested TOML never panics.

mod support;

use soma_template::{MAX_DOCUMENT_BYTES, parse_template};
use support::EXAMPLE;

const SCHEMA_LINE: &str = "schema = \"soma.template/v1alpha1\"\n";

const GARBAGE: &[&str] = &[
    "",
    " ",
    "=",
    "[",
    "[[",
    "]]",
    "[a.b",
    "a = ",
    "a = \"",
    "a = '''",
    "a = [1, ",
    "a = {",
    "a = { b = ",
    "schema = \"soma.template/v1alpha1\"\nschema = \"again\"",
    "schema = \"soma.template/v1alpha1\"\n[workload]\n[workload]",
    "schema = \"soma.template/v1alpha1\"\nname = 1e999",
    "schema = \"soma.template/v1alpha1\"\nname = 9223372036854775808",
    "schema = \"soma.template/v1alpha1\"\nname = -9223372036854775809",
    "schema = \"soma.template/v1alpha1\"\nname = 1979-05-27T07:32:00Z",
    "schema = \"soma.template/v1alpha1\"\nname = \"\\u0000\"",
    "schema = \"soma.template/v1alpha1\"\nname = \"\\UFFFFFFFF\"",
    "schema = \"soma.template/v1alpha1\"\nmodules = [[[[[[[[[[]]]]]]]]]]",
    "\u{feff}schema = \"soma.template/v1alpha1\"",
    "schema = \"soma.template/v1alpha1\"\r\nname = \"x\"\r\n",
    "schema = \"soma.template/v1alpha1\"\nname = \"x\"\n[[environment]]\nname = \"A\"\n[environment]\n",
];

#[test]
fn garbage_documents_never_panic() {
    for text in GARBAGE {
        let _ = parse_template(text.as_bytes());
    }
}

#[test]
fn every_prefix_of_the_example_never_panics() {
    let bytes = EXAMPLE.as_bytes();
    for length in 0..bytes.len() {
        let _ = parse_template(&bytes[..length]);
    }
}

#[test]
fn every_single_byte_mutation_of_the_example_never_panics() {
    let bytes = EXAMPLE.as_bytes();
    for index in 0..bytes.len() {
        for replacement in [0_u8, b'"', b'[', b']', b'=', b'\n', b'#', b'\\', 0x80, 0xff] {
            let mut mutated = bytes.to_vec();
            mutated[index] = replacement;
            let _ = parse_template(&mutated);
        }
    }
}

#[test]
fn deeply_nested_inline_tables_never_panic() {
    for depth in [64_usize, 1_024, 8_192] {
        let mut text = format!("{SCHEMA_LINE}name = ");
        text.push_str(&"{ a = ".repeat(depth));
        text.push('1');
        text.push_str(&" }".repeat(depth));
        assert!(text.len() <= MAX_DOCUMENT_BYTES);
        assert!(parse_template(text.as_bytes()).is_err());
    }
    let mut arrays = format!("{SCHEMA_LINE}name = ");
    arrays.push_str(&"[".repeat(20_000));
    arrays.push_str(&"]".repeat(20_000));
    assert!(parse_template(arrays.as_bytes()).is_err());
}

#[test]
fn pathological_but_bounded_documents_never_panic() {
    use std::fmt::Write as _;
    let mut many_keys = SCHEMA_LINE.to_owned();
    for index in 0..10_000 {
        writeln!(many_keys, "k{index} = 1").expect("String write cannot fail");
    }
    assert!(parse_template(many_keys.as_bytes()).is_err());
    let mut many_tables = format!("{SCHEMA_LINE}name = \"x\"\n");
    for _ in 0..10_000 {
        many_tables.push_str("[[environment]]\nname = \"A\"\nvalue = \"b\"\n");
    }
    let bytes = many_tables.as_bytes();
    let bounded = &bytes[..bytes.len().min(MAX_DOCUMENT_BYTES)];
    let _ = parse_template(bounded);
    let _ = parse_template(bytes);
}
