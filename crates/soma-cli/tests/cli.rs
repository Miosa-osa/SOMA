#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::process::{Command, Output};

use serde_json::Value;

mod support;

use support::FakeRuntime;

const INSTANCE_ID: &str = "22222222222222222222222222222222";

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("valid JSON response")
}

#[test]
fn one_shot_uses_the_real_facade_receipt_and_proves_cleanup() {
    let runtime = FakeRuntime::install();
    let output = macos_soma(
        &runtime,
        &[
            "--format",
            "json",
            "run",
            "--instance-id",
            INSTANCE_ID,
            "--vcpus",
            "2",
            "--memory-mib",
            "2048",
            "--storage-mib",
            "4096",
            "--network",
            "denied",
            "--name",
            "node-evaluator",
            "node:22",
            "--",
            "/usr/local/bin/node",
            "--version",
        ],
    );

    assert_eq!(output.status.code(), Some(0));
    let response = json(&output);
    assert_eq!(response["result"]["instance_id"], INSTANCE_ID);
    assert_eq!(response["result"]["execution"]["exited"]["code"], 0);
    assert_eq!(response["receipt"]["backend"], "macos_virtualization");
    assert_eq!(
        response["receipt"]["isolation"]["value"],
        "hardware_virtual_machine"
    );
    assert_eq!(response["receipt"]["requested_shape"]["vcpu_count"], 2);
    assert_eq!(
        response["receipt"]["effective_shape"]["vcpu_count"]["value"],
        2
    );
    assert_eq!(
        response["receipt"]["effective_network"]["egress"]["value"],
        "denied"
    );
    assert_eq!(
        response["receipt"]["effective_network"]["attachment"]["value"],
        "detached"
    );
    assert_eq!(response["receipt"]["cleanup"]["machine"], "complete");
    assert_eq!(response["receipt"]["machine_name"], "node-evaluator");

    let log = runtime.log();
    assert!(log.contains("<image>\n<pull>"));
    assert!(log.contains("<--cpus>\n<2>"));
    assert!(log.contains("<--network>\n<none>"));
    assert!(log.contains("<delete>\n<--force>\n<soma-22222222222222222222222222222222>"));
}

#[test]
fn binary_guest_output_is_exact_in_human_and_json_modes() {
    let runtime = FakeRuntime::install();
    let human = macos_soma(&runtime, &["run", "node:22", "--", "/bin/binary"]);
    assert_eq!(human.status.code(), Some(0));
    assert_eq!(human.stdout, [0xff, 0x00, b'A']);
    assert_eq!(human.stderr, [0xfe, b'B']);

    let machine = macos_soma(
        &runtime,
        &["--format", "json", "run", "node:22", "--", "/bin/binary"],
    );
    let response = json(&machine);
    assert_eq!(response["result"]["stdout"]["data"], "/wBB");
    assert_eq!(response["result"]["stderr"]["data"], "/kI=");
}

#[test]
fn command_failures_retain_output_receipt_and_cleanup_evidence() {
    let runtime = FakeRuntime::install();
    let failed = macos_soma(
        &runtime,
        &["--format", "json", "run", "node:22", "--", "/bin/fail"],
    );
    let timed_out = macos_soma(
        &runtime,
        &[
            "--format",
            "json",
            "run",
            "--timeout-ms",
            "10",
            "node:22",
            "--",
            "/bin/slow",
        ],
    );
    let limited = macos_soma(
        &runtime,
        &[
            "--format",
            "json",
            "run",
            "--max-output-bytes",
            "4",
            "node:22",
            "--",
            "/bin/noisy",
        ],
    );

    assert_command_failure(&failed, 10, "guest_nonzero", "exited");
    assert_command_failure(&timed_out, 124, "guest_timeout", "timed_out");
    assert_command_failure(&limited, 73, "output_limit", "output_limit_exceeded");
}

#[test]
fn managed_lifecycle_survives_independent_cli_processes() {
    let runtime = FakeRuntime::install();
    let launch = macos_soma(
        &runtime,
        &[
            "--format",
            "json",
            "machine",
            "launch",
            "--instance-id",
            INSTANCE_ID,
            "ubuntu:24.04",
        ],
    );
    assert_eq!(launch.status.code(), Some(0));
    assert_eq!(json(&launch)["result"]["state"], "ready");

    let executed = managed(&runtime, "exec", &["--", "/bin/true"]);
    assert_eq!(executed.status.code(), Some(0));
    assert_eq!(
        json(&executed)["result"]["stdout"]["data"],
        "ZXhlYyBmaXh0dXJlCg=="
    );

    let inspected = managed(&runtime, "inspect", &[]);
    assert_eq!(inspected.status.code(), Some(0));
    assert_eq!(json(&inspected)["result"]["state"], "ready");

    let stopped = managed(&runtime, "stop", &[]);
    assert_eq!(stopped.status.code(), Some(0));
    assert_eq!(json(&stopped)["result"]["state"], "stopped");
}

#[test]
fn launch_resource_mismatch_rolls_back_and_reports_backend_failure() {
    let runtime = FakeRuntime::install();
    let instance = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let output = macos_soma(
        &runtime,
        &[
            "--format",
            "json",
            "machine",
            "launch",
            "--instance-id",
            instance,
            "ubuntu:24.04",
        ],
    );

    assert_eq!(output.status.code(), Some(74));
    assert_eq!(json(&output)["error"]["code"], "backend_failure");
    let log = runtime.log();
    assert!(
        log.rfind("<delete>").expect("rollback delete") > log.rfind("<inspect>").expect("inspect")
    );
}

#[test]
fn runtime_failures_are_redacted() {
    let runtime = FakeRuntime::install();
    let missing = runtime
        .executable()
        .with_file_name("private-missing-runtime");
    let output = Command::new(env!("CARGO_BIN_EXE_soma"))
        .args(["--format", "json", "--backend", "macos", "--runtime"])
        .arg(&missing)
        .args([
            "run",
            "private.invalid/customer-image:secret",
            "--",
            "/bin/true",
        ])
        .output()
        .expect("execute soma");

    assert_eq!(output.status.code(), Some(76));
    let rendered = String::from_utf8(output.stdout).expect("UTF-8");
    assert!(!rendered.contains("private-missing-runtime"));
    assert!(!rendered.contains("customer-image"));
    assert_eq!(
        serde_json::from_str::<Value>(&rendered).expect("JSON")["error"]["code"],
        "backend_unavailable"
    );
}

fn macos_soma(runtime: &FakeRuntime, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soma"))
        .args(["--backend", "macos", "--runtime"])
        .arg(runtime.executable())
        .arg("--state-root")
        .arg(runtime.state_root())
        .args(arguments)
        .output()
        .expect("execute soma with fake Apple runtime")
}

fn managed(runtime: &FakeRuntime, verb: &str, suffix: &[&str]) -> Output {
    let mut arguments = vec![
        "--format",
        "json",
        "machine",
        verb,
        "--instance-id",
        INSTANCE_ID,
    ];
    arguments.extend_from_slice(suffix);
    macos_soma(runtime, &arguments)
}

fn assert_command_failure(output: &Output, exit: i32, code: &str, status: &str) {
    assert_eq!(output.status.code(), Some(exit));
    let response = json(output);
    assert_eq!(response["error"]["code"], code);
    if status == "exited" {
        assert!(response["result"]["execution"].get(status).is_some());
    } else {
        assert_eq!(response["result"]["execution"], status);
    }
    assert_eq!(response["receipt"]["cleanup"]["machine"], "complete");
}
