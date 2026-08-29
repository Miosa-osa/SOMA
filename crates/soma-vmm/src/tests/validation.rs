use crate::{
    Argument, CommandError, DiskBytes, Execute, GenerationId, IdError, InstanceId, MemoryBytes,
    OperationId, OutputBytes, Program, SpecError, TimeoutMillis, VcpuCount,
};

use super::fixtures::execution_limits;

#[test]
fn execute_rejects_an_unbounded_argument_set() {
    let argument = Argument::new(Vec::new()).expect("empty argument is valid");

    let failure = Execute::new(
        OperationId::new([9; 16]).expect("operation ID"),
        InstanceId::new([2; 16]).expect("instance ID"),
        Program::new(b"/usr/bin/true".to_vec()).expect("program"),
        vec![argument; 4_097],
        execution_limits(),
    )
    .expect_err("argument count must be bounded before execution");

    assert_eq!(failure, CommandError::TooLarge("argument count"));
}

#[test]
fn program_requires_an_absolute_nul_free_guest_path() {
    assert_eq!(
        Program::new(b"usr/bin/true".to_vec()),
        Err(CommandError::NotAbsolute("program"))
    );
    assert_eq!(
        Program::new(b"/usr/bin/tr\0ue".to_vec()),
        Err(CommandError::ContainsNul("program"))
    );
    assert!(Program::new(b"/usr/bin/true".to_vec()).is_ok());
}

#[test]
fn validated_newtypes_reject_ambiguous_or_unbounded_values() {
    assert_eq!(
        OperationId::new([0; 16]),
        Err(IdError::AllZero("operation"))
    );
    assert_eq!(InstanceId::new([0; 16]), Err(IdError::AllZero("instance")));
    assert_eq!(
        GenerationId::new([0; 32]),
        Err(IdError::AllZero("generation"))
    );
    assert_eq!(VcpuCount::new(0), Err(SpecError::Zero("vCPU count")));
    assert_eq!(MemoryBytes::new(0), Err(SpecError::Zero("memory bytes")));
    assert_eq!(
        DiskBytes::new(0),
        Err(SpecError::Zero("writable disk bytes"))
    );
    assert_eq!(
        TimeoutMillis::new(0),
        Err(CommandError::Zero("timeout milliseconds"))
    );
    assert_eq!(
        OutputBytes::new(16 * 1024 * 1024 + 1),
        Err(CommandError::TooLarge("output bytes"))
    );
}
