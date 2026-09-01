//! Reading one reply packet back on the supervisor's side.
//!
//! The worker encodes a borrowed receipt; a supervisor holds no receipt to borrow, so it
//! decodes into owned values. The two live beside each other so the encoder and the decoder
//! cannot drift: every form the worker can send is a form this names.
//!
//! Only what a supervisor acts on is recovered. A failure's phase, recovery and milestone trail
//! are kept as the text the worker sent, because a supervisor forwards them and never branches
//! on them, and decoding a value nobody reads is a way for the two sides to disagree silently.

use crate::{ExitStatus, FailureKind, OperationId};

use super::{ControlError, field};

/// What one reply packet says.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The Instance reached authenticated command readiness.
    Ready { operation_id: OperationId },
    /// One command completed, and its output is this many bytes on each stream.
    Executed {
        operation_id: OperationId,
        status: ExitStatus,
        stdout_bytes: u64,
        stderr_bytes: u64,
    },
    /// The machine stopped and proved its cleanup.
    Stopped {
        guest_acknowledged: bool,
        forced: bool,
    },
    /// The request was performed and failed, with the contract's typed reason.
    Failure { kind: FailureKind, detail: String },
    /// One bounded window of a completed command's output.
    Output(Vec<u8>),
    /// The filter reached its steady-state phase.
    Sealed,
    /// The request was not performed and Machine state did not change.
    Rejected(String),
}

impl Outcome {
    /// Parses one reply packet.
    ///
    /// # Errors
    ///
    /// Returns [`ControlError::UnknownRequest`] for a packet whose first word is not a reply
    /// form, or the [`ControlError`] naming the first field the packet does not satisfy.
    pub fn decode(text: &str) -> Result<Self, ControlError> {
        let (head, rest) = text.split_once(' ').unwrap_or((text, ""));
        match head {
            "sealed" => Ok(Self::Sealed),
            "rejected" => Ok(Self::Rejected(rest.to_owned())),
            "output" => field::bytes(Some(rest.trim()), "output").map(Self::Output),
            "ready" => Ok(Self::Ready {
                operation_id: operation(rest)?,
            }),
            "executed" => executed(rest),
            "stopped" => stopped(rest),
            "failure" => failure(rest),
            _ => Err(ControlError::UnknownRequest),
        }
    }
}

/// The value of one `name=value` field of a reply.
fn value<'a>(text: &'a str, name: &'static str) -> Result<&'a str, ControlError> {
    text.split_whitespace()
        .find_map(|token| token.strip_prefix(name)?.strip_prefix('='))
        .ok_or(ControlError::MissingField(name))
}

fn operation(text: &str) -> Result<OperationId, ControlError> {
    let bytes = field::identifier(Some(value(text, "operation")?), "operation")?;
    OperationId::new(bytes).map_err(|_| ControlError::InvalidValue("operation"))
}

fn number(text: &str, name: &'static str) -> Result<u64, ControlError> {
    field::number(Some(value(text, name)?), name)
}

fn boolean(text: &str, name: &'static str) -> Result<bool, ControlError> {
    match value(text, name)? {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ControlError::InvalidValue(name)),
    }
}

fn executed(text: &str) -> Result<Outcome, ControlError> {
    let status = ExitStatus::from_token(value(text, "status")?)
        .ok_or(ControlError::InvalidValue("status"))?;
    Ok(Outcome::Executed {
        operation_id: operation(text)?,
        status,
        stdout_bytes: number(text, "stdout")?,
        stderr_bytes: number(text, "stderr")?,
    })
}

fn stopped(text: &str) -> Result<Outcome, ControlError> {
    Ok(Outcome::Stopped {
        guest_acknowledged: boolean(text, "acknowledged")?,
        forced: boolean(text, "forced")?,
    })
}

fn failure(text: &str) -> Result<Outcome, ControlError> {
    let kind =
        FailureKind::from_name(value(text, "kind")?).ok_or(ControlError::InvalidValue("kind"))?;
    Ok(Outcome::Failure {
        kind,
        detail: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DeclaredDevices, DiskBytes, Executed, Generation, GenerationId, InstanceId, Launch,
        Machine, MachineSpec, MemoryBytes, Stopped, VcpuCount,
        control::Reply,
        {CleanupEvidence, Milestone, Milestones},
    };

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

    /// Every form the worker can encode is a form the supervisor recovers the same values from.
    #[test]
    fn every_reply_the_worker_sends_decodes_to_what_it_meant() {
        let executed = Executed::new(
            operation(),
            instance(),
            ExitStatus::Code(0),
            b"soma-ok\n".to_vec(),
            Vec::new(),
        );
        assert_eq!(
            Outcome::decode(&Reply::Executed(&executed).encode()),
            Ok(Outcome::Executed {
                operation_id: operation(),
                status: ExitStatus::Code(0),
                stdout_bytes: 8,
                stderr_bytes: 0,
            })
        );

        let mut milestones = Milestones::default();
        milestones.push(Milestone::Stopped);
        let stopped = Stopped::new(
            operation(),
            instance(),
            true,
            false,
            CleanupEvidence::Complete,
            milestones,
        );
        assert_eq!(
            Outcome::decode(&Reply::Stopped(&stopped).encode()),
            Ok(Outcome::Stopped {
                guest_acknowledged: true,
                forced: false,
            })
        );

        let failure = Machine::new().launch(launch()).expect_err("no platform");
        let Ok(Outcome::Failure { kind, .. }) = Outcome::decode(&Reply::Failure(&failure).encode())
        else {
            panic!("a failure reply must decode as a failure");
        };
        assert_eq!(kind, crate::FailureKind::GenerationVerificationFailed);

        assert_eq!(
            Outcome::decode(&Reply::Sealed.encode()),
            Ok(Outcome::Sealed)
        );
        assert_eq!(
            Outcome::decode(&Reply::Output(b"\x00\xff").encode()),
            Ok(Outcome::Output(vec![0, 255]))
        );
        assert_eq!(
            Outcome::decode(&Reply::Rejected("unknown request").encode()),
            Ok(Outcome::Rejected("unknown request".to_owned()))
        );
    }

    #[test]
    fn a_reply_this_contract_does_not_define_is_refused() {
        assert_eq!(
            Outcome::decode("mounted /"),
            Err(ControlError::UnknownRequest)
        );
        assert_eq!(
            Outcome::decode("executed operation=01 status=code:0 stdout=0 stderr=0"),
            Err(ControlError::InvalidValue("operation"))
        );
        assert_eq!(
            Outcome::decode("stopped acknowledged=maybe forced=false"),
            Err(ControlError::InvalidValue("acknowledged"))
        );
        assert_eq!(
            Outcome::decode("failure kind=NoSuchKind"),
            Err(ControlError::InvalidValue("kind"))
        );
    }
}
