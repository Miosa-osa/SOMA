//! The command half of the probe codec.
//!
//! The parent sends these over the control socket to make the probe exercise one behaviour
//! the jail must admit or deny; [`crate::report`] carries what the probe reports back.

use crate::report::ReportError;

/// Commands the parent sends to the probe over the control socket.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeCommand {
    Exit(i32),
    /// Install the steady-state filter on top of the startup filter.
    Steady,
    /// Call `socket(2)`, which no phase allows.
    Socket,
    /// Issue `TUNSETIFF` on the KVM descriptor, which no phase allows.
    ForbiddenIoctl,
    /// Issue `KVM_GET_API_VERSION` on the KVM descriptor and reply with the value.
    KvmVersion,
    /// Issue `KVM_SET_USER_MEMORY_REGION` on the KVM descriptor and reply with the errno.
    ///
    /// The descriptor is the wrong kind for this request, so the kernel always rejects it.
    /// The probe therefore distinguishes only whether the seccomp filter admitted the request
    /// at all: a reply means admitted, a `SIGSYS` kill means denied.
    SetMemoryRegion,
    /// Spawn this many blocking threads and reply with how many succeeded.
    Threads(u32),
    /// Allocate and touch this many MiB.
    Allocate(u32),
    /// Attempt `execve`, which no phase allows.
    Exec,
    /// Attempt `openat(O_CREAT)` in `/` and reply with the errno.
    CreateFile,
}

impl ProbeCommand {
    #[must_use]
    pub fn encode(self) -> String {
        match self {
            Self::Exit(code) => format!("exit {code}"),
            Self::Steady => "steady".to_owned(),
            Self::Socket => "socket".to_owned(),
            Self::ForbiddenIoctl => "forbidden-ioctl".to_owned(),
            Self::KvmVersion => "kvm-version".to_owned(),
            Self::SetMemoryRegion => "set-memory-region".to_owned(),
            Self::Threads(count) => format!("threads {count}"),
            Self::Allocate(mib) => format!("allocate {mib}"),
            Self::Exec => "exec".to_owned(),
            Self::CreateFile => "create-file".to_owned(),
        }
    }

    /// Parses one command packet.
    ///
    /// # Errors
    ///
    /// Returns [`ReportError::UnknownCommand`] or [`ReportError::InvalidValue`].
    pub fn decode(text: &str) -> Result<Self, ReportError> {
        let mut parts = text.trim_end().splitn(2, ' ');
        let word = parts.next().unwrap_or_default();
        let argument = parts.next();
        let parse = |argument: Option<&str>| {
            argument
                .ok_or(ReportError::InvalidValue)?
                .parse::<i64>()
                .map_err(|_| ReportError::InvalidValue)
        };
        let unsigned = |argument: Option<&str>| {
            u32::try_from(parse(argument)?).map_err(|_| ReportError::InvalidValue)
        };
        match word {
            "exit" => Ok(Self::Exit(
                i32::try_from(parse(argument)?).map_err(|_| ReportError::InvalidValue)?,
            )),
            "steady" => Ok(Self::Steady),
            "socket" => Ok(Self::Socket),
            "forbidden-ioctl" => Ok(Self::ForbiddenIoctl),
            "kvm-version" => Ok(Self::KvmVersion),
            "set-memory-region" => Ok(Self::SetMemoryRegion),
            "threads" => Ok(Self::Threads(unsigned(argument)?)),
            "allocate" => Ok(Self::Allocate(unsigned(argument)?)),
            "exec" => Ok(Self::Exec),
            "create-file" => Ok(Self::CreateFile),
            _ => Err(ReportError::UnknownCommand),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        let commands = [
            ProbeCommand::Exit(3),
            ProbeCommand::Steady,
            ProbeCommand::Socket,
            ProbeCommand::ForbiddenIoctl,
            ProbeCommand::KvmVersion,
            ProbeCommand::SetMemoryRegion,
            ProbeCommand::Threads(4),
            ProbeCommand::Allocate(64),
            ProbeCommand::Exec,
            ProbeCommand::CreateFile,
        ];
        for command in commands {
            assert_eq!(ProbeCommand::decode(&command.encode()), Ok(command));
        }
        assert_eq!(
            ProbeCommand::decode("mount"),
            Err(ReportError::UnknownCommand)
        );
        assert_eq!(
            ProbeCommand::decode("threads x"),
            Err(ReportError::InvalidValue)
        );
    }
}
