use std::process::{Command, Output};

use serde_json::Value;

fn soma(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soma"))
        .args(arguments)
        .output()
        .expect("execute soma")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("valid JSON response")
}

#[test]
fn version_reports_the_alpha_contract_without_claiming_kvm_readiness() {
    let output = soma(&["--format", "json", "version"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let response = json(&output);
    assert_eq!(response["schema"], "soma.cli.v1");
    assert_eq!(response["command"], "version");
    assert_eq!(response["result"]["version"], "1.0.0-alpha.1");
    assert_eq!(response["result"]["production_ready"], false);
    assert_eq!(response["result"]["native_kvm_lifecycle"], "unavailable");
    assert!(response["receipt"].is_null());
}

#[test]
fn parser_failures_are_machine_readable_without_echoing_values() {
    let secret = "private-image-that-must-not-echo";
    let output = soma(&[
        "--format",
        "json",
        "run",
        secret,
        "--instance-id",
        "not-an-instance-id",
    ]);

    assert_eq!(output.status.code(), Some(2));
    let rendered = String::from_utf8(output.stdout).expect("UTF-8 output");
    assert!(!rendered.contains(secret));
    let response: Value = serde_json::from_str(&rendered).expect("JSON");
    assert_eq!(response["error"]["code"], "usage");
}

#[test]
fn explicit_kvm_without_a_prepared_generation_reports_an_unavailable_capability() {
    let output = soma(&[
        "--format",
        "json",
        "--backend",
        "kvm",
        "run",
        "node:22",
        "--",
        "/usr/local/bin/node",
        "--version",
    ]);
    let response = json(&output);

    // The KVM lifecycle exists now, so the honest refusal is that this host has prepared no
    // Generation for the image, not that the Backend is unsupported. A host that has prepared
    // one is exercised by the ignored live tests rather than by this contract.
    assert_eq!(output.status.code(), Some(76));
    assert!(response["result"].is_null());
    assert_eq!(response["error"]["code"], "backend_unavailable");
}
