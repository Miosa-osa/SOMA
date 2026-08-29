//! Phase-derived seccomp filters: policy tables, the BPF assembler, and the Linux installer.

mod bpf;
mod denied;
mod ioctls;
mod policy;
#[cfg(test)]
mod tests;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod install;

pub use bpf::{FilterProgram, Instruction};
pub use denied::NEVER_ALLOWED;
pub use ioctls::{IoctlPhases, IoctlRule, STEADY_REQUESTS, TUNSETIFF};
pub use policy::{
    CLONE_NAMESPACE_MASK, CLONE_THREAD, Need, PROT_EXEC, Provenance, STARTUP_ONLY, Source,
    SyscallRule,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use install::{SeccompError, install_filter};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use install::{install_sock_filters, to_sock_filters};

use bpf::{
    AUDIT_ARCH_X86_64, DATA_ARCH, DATA_NR, RET_ALLOW, RET_ERRNO, RET_KILL_PROCESS, arg_high,
    arg_low,
};

/// The two filter phases of one VMM life.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// From `execveat` until restore is complete.
    Startup,
    /// After restore; setup-only syscalls and ioctls are gone.
    SteadyState,
}

/// How a syscall's arguments are checked before it is admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArgCheck {
    Any,
    /// Fail the call with this errno instead of killing.
    Errno(u16),
    /// The argument's high word must be zero and its low word one of `values`.
    ArgEqualsAny {
        index: u8,
        values: &'static [u32],
    },
    /// The argument's low word must contain no `forbidden` bit and, if `required` is nonzero,
    /// at least one `required` bit.
    Flags {
        index: u8,
        forbidden: u32,
        required: u32,
    },
    /// The `ioctl` request must be in the phase's [`ioctl_rules`].
    IoctlAllowlist,
}

/// The full syscall table.
#[must_use]
pub fn syscall_rules() -> &'static [SyscallRule] {
    policy::RULES
}

/// The full ioctl table.
#[must_use]
pub fn ioctl_rules() -> &'static [IoctlRule] {
    ioctls::IOCTL_RULES
}

/// Names of syscalls the steady-state filter drops.
#[must_use]
pub fn startup_only_syscalls() -> Vec<&'static str> {
    policy::RULES
        .iter()
        .filter(|rule| rule.steady.is_none())
        .map(|rule| rule.name)
        .collect()
}

/// Assembles the filter for `phase`.
///
/// # Panics
///
/// Panics if a policy table produces a block too long for an eight-bit jump, which the golden
/// tests rule out for the checked-in tables.
#[must_use]
pub fn program_for(phase: Phase) -> FilterProgram {
    let mut rules: Vec<(u32, ArgCheck)> = policy::RULES
        .iter()
        .filter_map(|rule| match phase {
            Phase::Startup => Some((rule.nr, rule.startup)),
            Phase::SteadyState => rule.steady.map(|check| (rule.nr, check)),
        })
        .collect();
    rules.sort_unstable_by_key(|(nr, _)| *nr);
    let requests = ioctls::requests_for(phase);

    let mut out = vec![
        Instruction::load(DATA_ARCH),
        Instruction::jump_eq(AUDIT_ARCH_X86_64, 1, 0),
        Instruction::ret(RET_KILL_PROCESS),
        Instruction::load(DATA_NR),
    ];
    for (nr, check) in rules {
        let body = body_for(check, &requests);
        let skip = u8::try_from(body.len()).expect("rule body fits an eight-bit jump");
        out.push(Instruction::jump_eq(nr, 0, skip));
        out.extend(body);
    }
    out.push(Instruction::ret(RET_KILL_PROCESS));
    FilterProgram::new(out)
}

fn body_for(check: ArgCheck, requests: &[u32]) -> Vec<Instruction> {
    match check {
        ArgCheck::Any => vec![Instruction::ret(RET_ALLOW)],
        ArgCheck::Errno(code) => vec![Instruction::ret(RET_ERRNO | u32::from(code))],
        ArgCheck::ArgEqualsAny { index, values } => equals_any(index, values),
        ArgCheck::IoctlAllowlist => equals_any(1, requests),
        ArgCheck::Flags {
            index,
            forbidden,
            required,
        } => flags(index, forbidden, required),
    }
}

/// `[LD hi][JEQ 0 ? kill][LD lo][JEQ v_i ? allow]...[RET KILL][RET ALLOW]`.
fn equals_any(index: u8, values: &[u32]) -> Vec<Instruction> {
    let count = u8::try_from(values.len()).expect("allowlist fits an eight-bit jump");
    let mut body = vec![
        Instruction::load(arg_high(index)),
        Instruction::jump_eq(0, 0, count + 1),
        Instruction::load(arg_low(index)),
    ];
    for (position, value) in values.iter().enumerate() {
        let offset = count - u8::try_from(position).expect("position below count");
        body.push(Instruction::jump_eq(*value, offset, 0));
    }
    body.push(Instruction::ret(RET_KILL_PROCESS));
    body.push(Instruction::ret(RET_ALLOW));
    body
}

/// `[LD lo][JSET forbidden ? kill][JSET required ? allow : kill][RET KILL][RET ALLOW]`.
fn flags(index: u8, forbidden: u32, required: u32) -> Vec<Instruction> {
    if required == 0 {
        vec![
            Instruction::load(arg_low(index)),
            Instruction::jump_set(forbidden, 1, 0),
            Instruction::ret(RET_ALLOW),
            Instruction::ret(RET_KILL_PROCESS),
        ]
    } else {
        vec![
            Instruction::load(arg_low(index)),
            Instruction::jump_set(forbidden, 1, 0),
            Instruction::jump_set(required, 1, 0),
            Instruction::ret(RET_KILL_PROCESS),
            Instruction::ret(RET_ALLOW),
        ]
    }
}
