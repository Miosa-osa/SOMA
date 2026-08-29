#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum ProcessExit {
    Success = 0,
    Usage = 2,
    GuestNonzero = 10,
    InvalidInput = 65,
    NotFound = 66,
    Conflict = 69,
    OutputLimit = 73,
    BackendFailure = 74,
    CleanupUncertain = 75,
    CapabilityUnavailable = 76,
    DoctorStrict = 77,
    UnsupportedBackend = 78,
    Software = 70,
    GuestTimeout = 124,
}

impl ProcessExit {
    pub const fn code(self) -> i32 {
        self as i32
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::ProcessExit;

    #[test]
    fn semantic_exit_codes_are_stable_and_distinct() {
        let codes = [
            ProcessExit::Success.code(),
            ProcessExit::Usage.code(),
            ProcessExit::GuestNonzero.code(),
            ProcessExit::InvalidInput.code(),
            ProcessExit::NotFound.code(),
            ProcessExit::Conflict.code(),
            ProcessExit::OutputLimit.code(),
            ProcessExit::BackendFailure.code(),
            ProcessExit::CleanupUncertain.code(),
            ProcessExit::CapabilityUnavailable.code(),
            ProcessExit::DoctorStrict.code(),
            ProcessExit::UnsupportedBackend.code(),
            ProcessExit::Software.code(),
            ProcessExit::GuestTimeout.code(),
        ];

        assert_eq!(codes.len(), codes.into_iter().collect::<HashSet<_>>().len());
        assert_eq!(ProcessExit::GuestTimeout.code(), 124);
    }
}
