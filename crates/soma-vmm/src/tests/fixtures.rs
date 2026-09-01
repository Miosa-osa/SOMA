use crate::{
    Argument, DiskBytes, Execute, ExecutionLimits, Generation, GenerationId, InstanceId, Launch,
    MachineSpec, MemoryBytes, OperationId, OutputBytes, Program, Stop, TimeoutMillis, VcpuCount,
};

pub(super) fn launch() -> Launch {
    Launch::new(
        OperationId::new([1; 16]).expect("operation ID"),
        InstanceId::new([2; 16]).expect("instance ID"),
        Generation::new(
            GenerationId::new([3; 32]).expect("generation ID"),
            machine_spec(),
            crate::DeclaredDevices::new(true, true),
        ),
    )
}

pub(super) fn machine_spec() -> MachineSpec {
    MachineSpec::new(
        VcpuCount::new(2).expect("vCPU count"),
        MemoryBytes::new(2 * 1024 * 1024 * 1024).expect("memory"),
        DiskBytes::new(20 * 1024 * 1024 * 1024).expect("disk"),
    )
}

pub(super) fn execute(operation_id: [u8; 16]) -> Execute {
    execute_with_output_limit(operation_id, 1024 * 1024)
}

pub(super) fn execute_with_output_limit(operation_id: [u8; 16], output_bytes: u64) -> Execute {
    Execute::new(
        OperationId::new(operation_id).expect("operation ID"),
        InstanceId::new([2; 16]).expect("instance ID"),
        Program::new(b"/usr/bin/true".to_vec()).expect("program"),
        vec![Argument::new(b"--version".to_vec()).expect("argument")],
        ExecutionLimits::new(
            TimeoutMillis::new(5_000).expect("timeout"),
            OutputBytes::new(output_bytes).expect("output limit"),
        ),
    )
    .expect("bounded execute request")
}

pub(super) fn execution_limits() -> ExecutionLimits {
    ExecutionLimits::new(
        TimeoutMillis::new(5_000).expect("timeout"),
        OutputBytes::new(1024 * 1024).expect("output limit"),
    )
}

pub(super) fn stop(operation_id: [u8; 16]) -> Stop {
    Stop::new(
        OperationId::new(operation_id).expect("operation ID"),
        InstanceId::new([2; 16]).expect("instance ID"),
    )
}
