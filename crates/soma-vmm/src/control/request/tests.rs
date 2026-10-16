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
        Request::Execute(execute()),
        Request::Stop(Stop::new(operation(), instance())),
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
