#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use soma_macos::{
    ControlLimits, CreateMachine, ExecuteCommand, ExecutionLimits, ExecutionStatus, GuestCommand,
    ImageReference, InstanceId, MacOsBackend, MachineShape, NetworkAttachment, NetworkPolicy,
};

#[test]
#[ignore = "requires Apple container 1.3.0 and a cached or reachable ubuntu:24.04 image"]
fn denied_network_has_no_attachment_route_or_dns() {
    let home = std::env::var_os("HOME").expect("macOS home directory");
    let runtime = PathBuf::from(home)
        .join("Library/Application Support/SOMA/apple-container/1.3.0/bin/container");
    let backend = MacOsBackend::with_executable(runtime);
    backend.probe().expect("pinned runtime is ready");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let instance = InstanceId::new(format!("{nanos:032x}")).expect("unique Instance ID");
    let control = ControlLimits::new(30_000, 1_048_576).expect("control limits");
    let create = CreateMachine::new(
        instance.clone(),
        ImageReference::new("ubuntu:24.04").expect("image"),
        MachineShape::new(1, 1_073_741_824).expect("shape"),
        GuestCommand::new("/bin/sleep", ["infinity"]).expect("keeper"),
        control,
    )
    .with_network_policy(NetworkPolicy::Denied);

    backend.create(&create).expect("create denied-network VM");
    let scenario = (|| {
        backend
            .start(instance.clone(), control)
            .map_err(|error| format!("start failed: {error:?}"))?;
        let inspection = backend
            .inspect(instance.clone(), control)
            .map_err(|error| format!("inspect failed: {error:?}"))?;
        if inspection.network_attachment() != Some(NetworkAttachment::Detached) {
            return Err("runtime did not prove a detached network".to_owned());
        }
        let command = GuestCommand::new(
            "/bin/sh",
            [
                "-c",
                "test \"$(wc -l < /proc/net/route)\" -eq 1 && ! getent hosts example.com",
            ],
        )
        .expect("network assertion command");
        let execution = backend
            .execute(&ExecuteCommand::new(
                instance.clone(),
                command,
                ExecutionLimits::new(5_000, 65_536).expect("execution limits"),
            ))
            .map_err(|error| format!("exec failed: {error:?}"))?;
        if execution.status() != (ExecutionStatus::Exited { code: 0 }) {
            return Err("guest observed a route or DNS resolution".to_owned());
        }
        Ok::<_, String>(())
    })();
    let cleanup = backend.delete(instance, control);

    scenario.expect("denied network is enforced");
    cleanup.expect("owned VM cleanup succeeds");
}

#[test]
#[ignore = "requires Apple container 1.3.0 and a cached or reachable ubuntu:24.04 image"]
fn allowed_network_has_an_attachment_and_route() {
    verify_attached_network(NetworkPolicy::Allowed);
}

#[test]
#[ignore = "requires Apple container 1.3.0 and a cached or reachable ubuntu:24.04 image"]
fn unspecified_network_observes_the_runtime_default_attachment() {
    verify_attached_network(NetworkPolicy::Unspecified);
}

fn verify_attached_network(policy: NetworkPolicy) {
    let home = std::env::var_os("HOME").expect("macOS home directory");
    let runtime = PathBuf::from(home)
        .join("Library/Application Support/SOMA/apple-container/1.3.0/bin/container");
    let backend = MacOsBackend::with_executable(runtime);
    backend.probe().expect("pinned runtime is ready");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let instance = InstanceId::new(format!("{nanos:032x}")).expect("unique Instance ID");
    let control = ControlLimits::new(30_000, 1_048_576).expect("control limits");
    let create = CreateMachine::new(
        instance.clone(),
        ImageReference::new("ubuntu:24.04").expect("image"),
        MachineShape::new(1, 1_073_741_824).expect("shape"),
        GuestCommand::new("/bin/sleep", ["infinity"]).expect("keeper"),
        control,
    )
    .with_network_policy(policy);

    backend.create(&create).expect("create attached-network VM");
    let scenario = (|| {
        backend
            .start(instance.clone(), control)
            .map_err(|error| format!("start failed: {error:?}"))?;
        let inspection = backend
            .inspect(instance.clone(), control)
            .map_err(|error| format!("inspect failed: {error:?}"))?;
        if inspection.network_attachment() != Some(NetworkAttachment::Attached) {
            return Err("runtime did not prove an attached network".to_owned());
        }
        let execution = backend
            .execute(&ExecuteCommand::new(
                instance.clone(),
                GuestCommand::new(
                    "/bin/sh",
                    ["-c", "test \"$(wc -l < /proc/net/route)\" -gt 1"],
                )
                .expect("route assertion command"),
                ExecutionLimits::new(5_000, 65_536).expect("execution limits"),
            ))
            .map_err(|error| format!("exec failed: {error:?}"))?;
        if execution.status() != (ExecutionStatus::Exited { code: 0 }) {
            return Err("guest route was unavailable".to_owned());
        }
        Ok::<_, String>(())
    })();
    let cleanup = backend.delete(instance, control);

    scenario.expect("attached network is observed");
    cleanup.expect("owned VM cleanup succeeds");
}
