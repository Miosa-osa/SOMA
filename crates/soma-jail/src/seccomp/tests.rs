//! Golden bytes, argument encoding, phase difference, and an interpreter over the programs.
//!
//! `tables.rs` holds the table-invariant tests that share the interpreter.

mod tables;

use super::{
    ArgCheck, Instruction, Phase, bpf, equals_any, flags, ioctls, policy, program_for,
    startup_only_syscalls,
};

const KILL: u32 = bpf::RET_KILL_PROCESS;
const ALLOW: u32 = bpf::RET_ALLOW;
const X32_BIT: u32 = 0x4000_0000;
const TUNSETIFF: u32 = ioctls::TUNSETIFF;
const KVM_RUN: u32 = 0xAE80;
const KVM_CREATE_VM: u32 = 0xAE01;

/// A minimal classic-BPF interpreter over `struct seccomp_data`.
fn evaluate(program: &super::FilterProgram, arch: u32, nr: u32, args: [u64; 6]) -> u32 {
    let word = |offset: u32| -> u32 {
        match offset {
            bpf::DATA_NR => nr,
            bpf::DATA_ARCH => arch,
            16..=63 => {
                let index = usize::try_from((offset - 16) / 8).expect("index");
                let value = args[index];
                if (offset - 16).is_multiple_of(8) {
                    u32::try_from(value & 0xffff_ffff).expect("low")
                } else {
                    u32::try_from(value >> 32).expect("high")
                }
            }
            _ => panic!("load outside seccomp_data: {offset}"),
        }
    };
    let instructions = program.instructions();
    let mut pc = 0usize;
    let mut acc = 0u32;
    loop {
        let instruction = instructions[pc];
        pc += 1;
        match instruction.code {
            bpf::LOAD_WORD => acc = word(instruction.k),
            bpf::JUMP_EQ => {
                pc += usize::from(if acc == instruction.k {
                    instruction.jt
                } else {
                    instruction.jf
                });
            }
            bpf::JUMP_SET => {
                pc += usize::from(if acc & instruction.k != 0 {
                    instruction.jt
                } else {
                    instruction.jf
                });
            }
            bpf::RETURN => return instruction.k,
            other => panic!("unknown opcode {other:#x}"),
        }
    }
}

pub(super) fn run(phase: Phase, nr: u32, args: [u64; 6]) -> u32 {
    evaluate(&program_for(phase), bpf::AUDIT_ARCH_X86_64, nr, args)
}

pub(super) fn nr(name: &str) -> u32 {
    policy::RULES
        .iter()
        .find(|rule| rule.name == name)
        .map_or_else(|| panic!("{name}"), |rule| rule.nr)
}

#[test]
fn programs_assemble_deterministically() {
    for phase in [Phase::Startup, Phase::SteadyState] {
        assert_eq!(program_for(phase), program_for(phase));
    }
    let startup = program_for(Phase::Startup);
    let steady = program_for(Phase::SteadyState);
    assert!(startup.len() > steady.len());
    assert_ne!(startup.fingerprint(), steady.fingerprint());
    println!(
        "GOLDEN startup {} {:#018x} steady {} {:#018x}",
        startup.len(),
        startup.fingerprint(),
        steady.len(),
        steady.fingerprint()
    );
    assert_eq!(
        (startup.len(), startup.fingerprint()),
        (222, 0x40b7_c33a_9001_c79b)
    );
    assert_eq!(
        (steady.len(), steady.fingerprint()),
        (135, 0xe748_c586_d587_7538)
    );
}

#[test]
fn golden_bytes_for_argument_checks() {
    let body = equals_any(1, &[KVM_RUN, 0x5421]);
    assert_eq!(
        body,
        vec![
            Instruction::load(28),
            Instruction::jump_eq(0, 0, 3),
            Instruction::load(24),
            Instruction::jump_eq(KVM_RUN, 2, 0),
            Instruction::jump_eq(0x5421, 1, 0),
            Instruction::ret(KILL),
            Instruction::ret(ALLOW),
        ]
    );
    let bytes = super::FilterProgram::new(body).bytes();
    assert_eq!(
        &bytes[..16],
        &[0x20, 0, 0, 0, 28, 0, 0, 0, 0x15, 0, 0, 3, 0, 0, 0, 0]
    );
    assert_eq!(
        flags(2, policy::PROT_EXEC, 0),
        vec![
            Instruction::load(32),
            Instruction::jump_set(policy::PROT_EXEC, 1, 0),
            Instruction::ret(ALLOW),
            Instruction::ret(KILL),
        ]
    );
    assert_eq!(
        flags(0, policy::CLONE_NAMESPACE_MASK, policy::CLONE_THREAD),
        vec![
            Instruction::load(16),
            Instruction::jump_set(policy::CLONE_NAMESPACE_MASK, 1, 0),
            Instruction::jump_set(policy::CLONE_THREAD, 1, 0),
            Instruction::ret(KILL),
            Instruction::ret(ALLOW),
        ]
    );
}

#[test]
fn wrong_architecture_and_x32_calls_are_killed() {
    let program = program_for(Phase::Startup);
    assert_eq!(evaluate(&program, 0x4000_003E, nr("read"), [0; 6]), KILL);
    assert_eq!(run(Phase::Startup, nr("read") | X32_BIT, [0; 6]), KILL);
    assert_eq!(run(Phase::Startup, nr("read"), [0; 6]), ALLOW);
}

#[test]
fn ioctl_requests_are_filtered_per_phase() {
    let request = |value: u64| [3, value, 0, 0, 0, 0];
    for phase in [Phase::Startup, Phase::SteadyState] {
        assert_eq!(run(phase, nr("ioctl"), request(u64::from(KVM_RUN))), ALLOW);
        assert_eq!(run(phase, nr("ioctl"), request(u64::from(TUNSETIFF))), KILL);
        assert_eq!(
            run(phase, nr("ioctl"), request(u64::from(KVM_RUN) | (1 << 32))),
            KILL
        );
    }
    assert_eq!(
        run(
            Phase::Startup,
            nr("ioctl"),
            request(u64::from(KVM_CREATE_VM))
        ),
        ALLOW
    );
    assert_eq!(
        run(
            Phase::SteadyState,
            nr("ioctl"),
            request(u64::from(KVM_CREATE_VM))
        ),
        KILL
    );
}

#[test]
fn executable_mappings_and_setup_calls_leave_in_steady_state() {
    let exec = [0, 4096, u64::from(policy::PROT_EXEC | 1), 0, 0, 0];
    let plain = [0, 4096, 3, 0, 0, 0];
    for name in ["mmap", "mprotect"] {
        assert_eq!(run(Phase::Startup, nr(name), exec), ALLOW);
        assert_eq!(run(Phase::SteadyState, nr(name), exec), KILL);
        assert_eq!(run(Phase::SteadyState, nr(name), plain), ALLOW);
    }
    for name in policy::STARTUP_ONLY {
        let args = if *name == "fcntl" {
            [0, 3, 0, 0, 0, 0]
        } else {
            [0; 6]
        };
        assert_eq!(run(Phase::Startup, nr(name), args), ALLOW, "{name}");
        assert_eq!(run(Phase::SteadyState, nr(name), args), KILL, "{name}");
    }
    assert_eq!(run(Phase::Startup, nr("prctl"), [38, 0, 0, 0, 0, 0]), ALLOW);
    assert_eq!(
        run(Phase::SteadyState, nr("prctl"), [38, 0, 0, 0, 0, 0]),
        KILL
    );
    assert_eq!(
        run(Phase::SteadyState, nr("prctl"), [15, 0, 0, 0, 0, 0]),
        ALLOW
    );
    for request in [1, 4, 22] {
        assert_eq!(
            run(Phase::Startup, nr("prctl"), [request, 0, 0, 0, 0, 0]),
            KILL
        );
    }
    assert_eq!(
        run(Phase::Startup, nr("fcntl"), [0, 1030, 0, 0, 0, 0]),
        ALLOW
    );
    assert_eq!(
        run(Phase::Startup, nr("fcntl"), [0, 1024, 0, 0, 0, 0]),
        KILL
    );
}

#[test]
fn clone_admits_threads_only_and_clone3_reports_enosys() {
    let thread = [u64::from(policy::CLONE_THREAD | 0x100), 0, 0, 0, 0, 0];
    let namespace = [u64::from(policy::CLONE_THREAD | 0x1000_0000), 0, 0, 0, 0, 0];
    let process = [0x11, 0, 0, 0, 0, 0];
    for phase in [Phase::Startup, Phase::SteadyState] {
        assert_eq!(run(phase, nr("clone"), thread), ALLOW);
        assert_eq!(run(phase, nr("clone"), namespace), KILL);
        assert_eq!(run(phase, nr("clone"), process), KILL);
        assert_eq!(
            run(phase, nr("clone3"), [0; 6]),
            bpf::RET_ERRNO | u32::from(policy::ENOSYS)
        );
    }
}

#[test]
fn phase_difference_is_exactly_the_documented_set() {
    let mut computed = startup_only_syscalls();
    computed.sort_unstable();
    let mut documented = policy::STARTUP_ONLY.to_vec();
    documented.sort_unstable();
    assert_eq!(computed, documented);
    let mut steady: Vec<&str> = ioctls::IOCTL_RULES
        .iter()
        .filter(|rule| rule.phases == ioctls::IoctlPhases::Both)
        .map(|rule| rule.name)
        .collect();
    steady.sort_unstable();
    let mut documented = ioctls::STEADY_REQUESTS.to_vec();
    documented.sort_unstable();
    assert_eq!(steady, documented);
    let tightened: Vec<&str> = policy::RULES
        .iter()
        .filter(|rule| rule.steady.is_some_and(|steady| steady != rule.startup))
        .map(|rule| rule.name)
        .collect();
    assert_eq!(tightened, ["mmap", "mprotect", "prctl"]);
    assert!(
        policy::RULES
            .iter()
            .any(|rule| rule.startup == ArgCheck::IoctlAllowlist)
    );
}
