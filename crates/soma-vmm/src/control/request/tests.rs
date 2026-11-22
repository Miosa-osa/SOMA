//! The `Request` unit tests: one encode/decode pair per request the machine protocol carries.
//!
//! They live apart from the type so the type stays inside the source budget.

use super::*;
use crate::DeclaredDevices;
use crate::control::OutputStream;
use crate::{
    Argument, DiskBytes, ExecutionLimits, Generation, GenerationId, InstanceId, MachineSpec,
    MemoryBytes, OperationId, OutputBytes, Program, TimeoutMillis, VcpuCount,
};

mod context;

fn operation() -> OperationId {
    OperationId::new([1; 16]).expect("operation")
}

fn instance() -> InstanceId {
    InstanceId::new([2; 16]).expect("instance")
}

fn launch() -> Launch {
    let machine = MachineSpec::new(
        VcpuCount::new(1).expect("vcpus"),
        MemoryBytes::new(1 << 30).expect("memory"),
        DiskBytes::new(1 << 32).expect("disk"),
    );
    let generation = Generation::new(
        GenerationId::new([3; 32]).expect("generation"),
        machine,
        DeclaredDevices::new(true, true),
    );
    Launch::new(operation(), instance(), generation)
}

fn execute() -> Execute {
    let limits = ExecutionLimits::new(
        TimeoutMillis::new(5_000).expect("timeout"),
        OutputBytes::new(65_536).expect("output"),
    );
    Execute::new(
        operation(),
        instance(),
        Program::new(b"/bin/node".to_vec()).expect("program"),
        vec![Argument::new(b"--version".to_vec()).expect("argument")],
        limits,
    )
    .expect("execute")
}

fn launch_with_hotplug(bytes: u64) -> Launch {
    let machine = MachineSpec::new(
        VcpuCount::new(1).expect("vcpus"),
        MemoryBytes::new(1 << 30).expect("memory"),
        DiskBytes::new(1 << 32).expect("disk"),
    )
    .with_hotplug(crate::HotplugBytes::new(bytes).expect("hotplug"));
    Launch::new(
        operation(),
        instance(),
        Generation::new(
            GenerationId::new([3; 32]).expect("generation"),
            machine,
            DeclaredDevices::new(true, true),
        ),
    )
}

#[test]
fn a_launch_without_a_region_encodes_exactly_as_before_and_one_with_it_round_trips() {
    let plain = Request::Launch(launch()).encode();
    assert_eq!(plain.split(' ').count(), 10, "{plain}");
    let region = Request::Launch(launch_with_hotplug(1 << 30));
    let encoded = region.encode();
    assert_eq!(encoded, format!("{plain} {}", 1_u64 << 30));
    assert_eq!(Request::decode(&encoded), Ok(region));
    assert!(
        matches!(
            Request::decode(&Request::Launch(launch()).encode()),
            Ok(Request::Launch(decoded)) if decoded.generation().machine().hotplug().is_none()
        ),
        "the old form decodes with no region"
    );
}

#[test]
fn a_malformed_or_extra_hotplug_field_is_refused() {
    let plain = Request::Launch(launch()).encode();
    for bad in [
        format!("{plain} 0"),
        format!("{plain} x"),
        format!("{plain} -1"),
        format!("{plain} 1024 1024"),
    ] {
        assert!(Request::decode(&bad).is_err(), "{bad}");
    }
}

#[test]
fn every_request_round_trips() {
    let requests = [
        Request::Attest,
        Request::Seal,
        Request::Shutdown(-1),
        Request::Launch(launch()),
        Request::Recover(launch(), [5; 32]),
        Request::Fork(launch(), [5; 32], InstanceId::new([9; 16]).unwrap()),
        Request::Execute(execute()),
        Request::Stop(Stop::new(operation(), instance())),
        Request::Pause(instance()),
        Request::Resume(instance()),
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        Request::Workspace {
            instance: instance(),
            attach: true,
        },
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        Request::Workspace {
            instance: instance(),
            attach: false,
        },
        Request::Output(
            OutputWindow::new(operation(), OutputStream::Stderr, 1024, 4096).expect("window"),
        ),
    ];
    for request in requests {
        let encoded = request.encode();
        assert!(encoded.is_ascii() && !encoded.contains('\n'), "{encoded:?}");
        assert_eq!(Request::decode(&encoded), Ok(request), "{encoded:?}");
    }
}

#[test]
fn decoding_names_the_field_it_refuses() {
    assert_eq!(Request::decode("mount"), Err(ControlError::UnknownRequest));
    assert_eq!(
        Request::decode("attest now"),
        Err(ControlError::TrailingField)
    );
    assert_eq!(
        Request::decode("shutdown"),
        Err(ControlError::MissingField("exit status"))
    );
    assert_eq!(
        Request::decode(&"seal ".repeat(MAX_REQUEST_BYTES)),
        Err(ControlError::TooLong)
    );
    let zero_operation = Request::Stop(Stop::new(operation(), instance()))
        .encode()
        .replace(&hex(&[1; 16]), &hex(&[0; 16]));
    assert_eq!(
        Request::decode(&zero_operation),
        Err(ControlError::InvalidValue("operation"))
    );
}

#[test]
fn a_relative_program_is_refused_by_the_contract_type() {
    let relative = format!(
        "execute {} {} 1 1 {}",
        hex(&[1; 16]),
        hex(&[2; 16]),
        hex(b"node")
    );
    assert_eq!(
        Request::decode(&relative),
        Err(ControlError::InvalidValue("program"))
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn terminal_requests_round_trip_binary_input_without_logging_it() {
    let request = Request::Pty(super::super::PtyRequest::new(
        operation(),
        instance(),
        soma::PtyOperation::Write {
            bytes: vec![0, 255, b'\n'],
        },
    ));
    let encoded = request.encode();
    assert!(encoded.is_ascii() && !encoded.contains('\n'));
    assert_eq!(Request::decode(&encoded), Ok(request));
}

#[test]
fn a_volume_launch_carries_the_certified_capacity_and_ordinary_launch_stays_unchanged() {
    use std::num::NonZeroU64;
    let normal = launch();
    let generation = normal.generation();
    let capacity = NonZeroU64::new(10 * 1024 * 1024 * 1024).unwrap();
    let persistent_launch = Launch::new(
        normal.operation_id(),
        normal.instance_id(),
        Generation::new(
            generation.id(),
            generation.machine(),
            generation.devices().with_volume_capacity(capacity),
        ),
    );
    let persistent = Request::Launch(persistent_launch.clone());
    let encoded = persistent.encode();
    assert_eq!(Request::decode(&encoded), Ok(persistent));
    assert!(encoded.starts_with("launch_volume 10737418240 "));
    assert!(
        Request::Launch(normal.clone())
            .encode()
            .starts_with("launch ")
    );
    assert!(Request::decode(&encoded.replacen("10737418240", "0", 1)).is_err());
    let source = InstanceId::new([9; 16]).unwrap();
    let fork = Request::Fork(persistent_launch, [7; 32], source);
    let encoded = fork.encode();
    assert!(encoded.starts_with("fork_volume 10737418240 "));
    assert_eq!(Request::decode(&encoded), Ok(fork));
    assert!(Request::decode(&encoded.replacen("10737418240", "0", 1)).is_err());
    assert!(
        Request::Fork(normal, [7; 32], source)
            .encode()
            .starts_with("fork ")
    );
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn checkpoint_disk_flags_preserve_legacy_requests_and_round_trip_a_volume() {
    const WIRE_OVERLAY: u8 = 1;
    const WIRE_OVERLAY_AND_VOLUME: u8 = 3;
    let source = crate::sandbox::CheckpointSource {
        instance_id: [1; 16],
        candidate_id: [2; 32],
        root_digest: soma_kvm::snapshot::Digest::from_bytes([3; 32]),
        contract: soma_kvm::MachineContract::V1,
        has_overlay: true,
        has_volume: false,
    };
    for (source, flags) in [
        (source, WIRE_OVERLAY),
        (
            crate::sandbox::CheckpointSource {
                has_volume: true,
                ..source
            },
            WIRE_OVERLAY_AND_VOLUME,
        ),
    ] {
        let request = Request::Checkpoint(source);
        let encoded = request.encode();
        assert_eq!(
            encoded.split_whitespace().last(),
            Some(flags.to_string().as_str())
        );
        assert_eq!(Request::decode(&encoded), Ok(request));
    }
}
