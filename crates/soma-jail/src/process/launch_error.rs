//! Typed launcher failures and the failure-plus-cleanup result of a launch.

use std::{error::Error, fmt};

use super::failure::ChildFailure;
use crate::{
    cgroup::CgroupError,
    descriptors::DescriptorError,
    evidence::{ExitReason, ProcessStatus},
    namespaces::NamespaceError,
    reconcile::Disposition,
    spec::SpecError,
};

/// Typed launcher failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    Spec(SpecError),
    /// The resource roles do not match the manifest order.
    ManifestMismatch,
    Cgroup(CgroupError),
    CgroupMembership(CgroupError),
    JailRoot(i32),
    Pipe(i32),
    Seal(DescriptorError),
    Clone(i32),
    IdMap(NamespaceError),
    Namespace(NamespaceError),
    Child(ChildFailure),
    /// The failure report was short or malformed.
    Report(i32),
    /// `/proc/<pid>/status` after exec did not show the expected identity and filter state.
    Status(ProcessStatus),
    /// The child died after `execveat` before its status could be confirmed.
    Died(ExitReason),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spec(error) => write!(formatter, "invalid jail specification: {error}"),
            Self::ManifestMismatch => write!(formatter, "resources do not match the manifest"),
            Self::Cgroup(error) => write!(formatter, "cgroup leaf: {error}"),
            Self::CgroupMembership(error) => write!(formatter, "cgroup membership: {error}"),
            Self::JailRoot(errno) => write!(formatter, "jail root directory: errno {errno}"),
            Self::Pipe(errno) => write!(formatter, "launcher pipe: errno {errno}"),
            Self::Seal(error) => write!(formatter, "descriptor plan: {error}"),
            Self::Clone(errno) => write!(formatter, "clone3: errno {errno}"),
            Self::IdMap(error) | Self::Namespace(error) => write!(formatter, "namespace: {error}"),
            Self::Child(failure) => write!(formatter, "{failure}"),
            Self::Report(errno) => {
                write!(formatter, "child failure report unreadable: errno {errno}")
            }
            Self::Status(status) => write!(formatter, "unexpected post-exec status {status:?}"),
            Self::Died(exit) => write!(formatter, "child died after exec: {exit}"),
        }
    }
}

impl Error for LaunchError {}

/// A launch error together with what cleanup achieved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchFailure {
    pub error: LaunchError,
    pub cleanup: Disposition,
}

impl fmt::Display for LaunchFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} (cleanup: {})", self.error, self.cleanup)
    }
}

impl Error for LaunchFailure {}
