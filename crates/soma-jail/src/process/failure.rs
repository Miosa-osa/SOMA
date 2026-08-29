//! The twelve-byte failure report the pre-exec child writes, and its typed steps.

use std::fmt;

use crate::{
    descriptors::DescriptorError, manifest::DescriptorKind, namespaces::RootStep,
    seccomp::SeccompError,
};

/// Which resource limit failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RlimitKind {
    Core,
    Nofile,
    Nproc,
    Fsize,
    AddressSpace,
}

/// The pre-exec step that failed in the child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChildStep {
    DeathSignal,
    /// The launcher closed the synchronization pipe before releasing the child.
    LauncherGone,
    SetGid,
    SetUid,
    Dumpable,
    Rlimit(RlimitKind),
    Root(RootStep),
    Seal(DescriptorError),
    Verify(DescriptorError),
    Seccomp(SeccompError),
    Exec,
}

/// One failed child step with the errno it observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChildFailure {
    pub step: ChildStep,
    pub errno: i32,
}

/// Size of the encoded failure report.
pub const REPORT_BYTES: usize = 12;

const RLIMITS: [RlimitKind; 5] = [
    RlimitKind::Core,
    RlimitKind::Nofile,
    RlimitKind::Nproc,
    RlimitKind::Fsize,
    RlimitKind::AddressSpace,
];
const ROOT_STEPS: [RootStep; 7] = [
    RootStep::OpenOldRoot,
    RootStep::MakeRootPrivate,
    RootStep::MountEmptyRoot,
    RootStep::EnterNewRoot,
    RootStep::PivotRoot,
    RootStep::DetachOldRoot,
    RootStep::ChdirRoot,
];
const KINDS: [DescriptorKind; 5] = [
    DescriptorKind::CharDevice,
    DescriptorKind::RegularFile,
    DescriptorKind::Socket,
    DescriptorKind::Fifo,
    DescriptorKind::AnonInode,
];

fn index_of<T: PartialEq>(table: &[T], value: &T) -> u32 {
    u32::try_from(
        table
            .iter()
            .position(|candidate| candidate == value)
            .unwrap_or(0),
    )
    .unwrap_or(0)
}

/// `(sub-tag, errno-or-kind, slot)`.
fn encode_descriptor(error: DescriptorError) -> (u32, i32, u32) {
    match error {
        DescriptorError::Missing { slot, errno } => (0, errno, slot),
        DescriptorError::Kind { slot, found } => {
            let kind = found.map_or(-1, |kind| {
                i32::try_from(index_of(&KINDS, &kind)).unwrap_or(-1)
            });
            (1, kind, slot)
        }
        DescriptorError::Device { slot } => (2, 0, slot),
        DescriptorError::NotSeqpacket { slot } => (3, 0, slot),
        DescriptorError::Unexpected { slot } => (4, 0, slot),
        DescriptorError::Dup { slot, errno } => (5, errno, slot),
        DescriptorError::CloseRange(errno) => (6, errno, 0),
    }
}

fn decode_descriptor(sub: u32, value: i32, slot: u32) -> Option<DescriptorError> {
    Some(match sub {
        0 => DescriptorError::Missing { slot, errno: value },
        1 => DescriptorError::Kind {
            slot,
            found: usize::try_from(value)
                .ok()
                .and_then(|index| KINDS.get(index).copied()),
        },
        2 => DescriptorError::Device { slot },
        3 => DescriptorError::NotSeqpacket { slot },
        4 => DescriptorError::Unexpected { slot },
        5 => DescriptorError::Dup { slot, errno: value },
        6 => DescriptorError::CloseRange(value),
        _ => return None,
    })
}

impl ChildFailure {
    /// Encodes as `[tag][errno][detail]` little-endian words; allocation-free.
    #[must_use]
    pub fn encode(self) -> [u8; REPORT_BYTES] {
        let (tag, errno, detail): (u32, i32, u32) = match self.step {
            ChildStep::DeathSignal => (0, self.errno, 0),
            ChildStep::LauncherGone => (1, self.errno, 0),
            ChildStep::SetGid => (2, self.errno, 0),
            ChildStep::SetUid => (3, self.errno, 0),
            ChildStep::Dumpable => (4, self.errno, 0),
            ChildStep::Rlimit(kind) => (10 + index_of(&RLIMITS, &kind), self.errno, 0),
            ChildStep::Root(step) => (20 + index_of(&ROOT_STEPS, &step), self.errno, 0),
            ChildStep::Seal(error) => {
                let (sub, value, slot) = encode_descriptor(error);
                (30 + sub, value, slot)
            }
            ChildStep::Verify(error) => {
                let (sub, value, slot) = encode_descriptor(error);
                (40 + sub, value, slot)
            }
            ChildStep::Seccomp(SeccompError::NoNewPrivs(errno)) => (50, errno, 0),
            ChildStep::Seccomp(SeccompError::Install(errno)) => (51, errno, 0),
            ChildStep::Seccomp(SeccompError::ThreadSyncFailed(tid)) => {
                (52, 0, u32::try_from(tid).unwrap_or(u32::MAX))
            }
            ChildStep::Exec => (60, self.errno, 0),
        };
        let mut bytes = [0u8; REPORT_BYTES];
        bytes[..4].copy_from_slice(&tag.to_le_bytes());
        bytes[4..8].copy_from_slice(&errno.to_le_bytes());
        bytes[8..].copy_from_slice(&detail.to_le_bytes());
        bytes
    }

    /// Decodes [`Self::encode`] output; `None` for an unknown tag.
    #[must_use]
    pub fn decode(bytes: [u8; REPORT_BYTES]) -> Option<Self> {
        let tag = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let errno = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let detail = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        // Descriptor sub-tags 1 to 4 and the thread-sync tag carry no errno in that word.
        let carries_errno = !matches!(tag, 31..=34 | 41..=44 | 52);
        let step = match tag {
            0 => ChildStep::DeathSignal,
            1 => ChildStep::LauncherGone,
            2 => ChildStep::SetGid,
            3 => ChildStep::SetUid,
            4 => ChildStep::Dumpable,
            10..=14 => ChildStep::Rlimit(RLIMITS[usize::try_from(tag - 10).ok()?]),
            20..=26 => ChildStep::Root(ROOT_STEPS[usize::try_from(tag - 20).ok()?]),
            30..=36 => ChildStep::Seal(decode_descriptor(tag - 30, errno, detail)?),
            40..=46 => ChildStep::Verify(decode_descriptor(tag - 40, errno, detail)?),
            50 => ChildStep::Seccomp(SeccompError::NoNewPrivs(errno)),
            51 => ChildStep::Seccomp(SeccompError::Install(errno)),
            52 => ChildStep::Seccomp(SeccompError::ThreadSyncFailed(i64::from(detail))),
            60 => ChildStep::Exec,
            _ => return None,
        };
        Some(Self {
            step,
            errno: if carries_errno { errno } else { 0 },
        })
    }
}

impl fmt::Display for ChildFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "child step {:?} failed with errno {}",
            self.step, self.errno
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_reports_round_trip() {
        let steps = [
            ChildStep::DeathSignal,
            ChildStep::LauncherGone,
            ChildStep::SetGid,
            ChildStep::SetUid,
            ChildStep::Dumpable,
            ChildStep::Rlimit(RlimitKind::AddressSpace),
            ChildStep::Root(RootStep::PivotRoot),
            ChildStep::Seal(DescriptorError::Dup { slot: 7, errno: 9 }),
            ChildStep::Seal(DescriptorError::CloseRange(22)),
            ChildStep::Verify(DescriptorError::Missing { slot: 3, errno: 9 }),
            ChildStep::Verify(DescriptorError::Kind {
                slot: 4,
                found: Some(DescriptorKind::Fifo),
            }),
            ChildStep::Verify(DescriptorError::Kind {
                slot: 4,
                found: None,
            }),
            ChildStep::Verify(DescriptorError::Device { slot: 3 }),
            ChildStep::Verify(DescriptorError::NotSeqpacket { slot: 5 }),
            ChildStep::Verify(DescriptorError::Unexpected { slot: 40 }),
            ChildStep::Seccomp(SeccompError::NoNewPrivs(1)),
            ChildStep::Seccomp(SeccompError::Install(22)),
            ChildStep::Seccomp(SeccompError::ThreadSyncFailed(77)),
            ChildStep::Exec,
        ];
        for step in steps {
            let failure = ChildFailure { step, errno: 13 };
            let decoded = ChildFailure::decode(failure.encode()).expect("decodes");
            assert_eq!(decoded.step, step);
            let expected_errno = match step {
                ChildStep::Seal(inner) | ChildStep::Verify(inner) => match inner {
                    DescriptorError::Missing { errno, .. }
                    | DescriptorError::Dup { errno, .. }
                    | DescriptorError::CloseRange(errno) => errno,
                    _ => 0,
                },
                ChildStep::Seccomp(
                    SeccompError::NoNewPrivs(errno) | SeccompError::Install(errno),
                ) => errno,
                ChildStep::Seccomp(SeccompError::ThreadSyncFailed(_)) => 0,
                _ => 13,
            };
            assert_eq!(decoded.errno, expected_errno, "{step:?}");
        }
        assert_eq!(ChildFailure::decode([0xff; REPORT_BYTES]), None);
    }
}
