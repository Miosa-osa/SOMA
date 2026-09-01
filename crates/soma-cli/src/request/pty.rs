//! Turning the five terminal subcommands into the facade's terminal request.

use soma::{InstanceId, PtyMachineRequest, PtyOperation};

use crate::cli::{PtyArgs, PtyCommand, PtyTargetArgs};

use super::{PreparedOperation, RequestError, operation_id};

pub(super) fn prepare(arguments: PtyArgs) -> Result<PreparedOperation, RequestError> {
    let (target, operation) = match arguments.command {
        PtyCommand::Open(arguments) => (
            arguments.target,
            PtyOperation::Open {
                columns: arguments.columns,
                rows: arguments.rows,
            },
        ),
        PtyCommand::Resize(arguments) => (
            arguments.target,
            PtyOperation::Resize {
                columns: arguments.columns,
                rows: arguments.rows,
            },
        ),
        PtyCommand::Write(arguments) => {
            let bytes = read_host_file(&arguments.input_file)?;
            (arguments.target, PtyOperation::Write { bytes })
        }
        PtyCommand::Read(arguments) => (
            arguments.target,
            PtyOperation::Read {
                wait_millis: arguments.wait_ms,
            },
        ),
        PtyCommand::Close(target) => (target, PtyOperation::Close),
    };
    operation.check().map_err(|_| RequestError::Terminal)?;
    let instance_id = instance(&target)?;
    Ok(PreparedOperation::Pty {
        request: PtyMachineRequest::new(
            operation_id(target.operation_id)?,
            instance_id.clone(),
            operation,
        ),
        instance_id,
    })
}

fn instance(target: &PtyTargetArgs) -> Result<InstanceId, RequestError> {
    InstanceId::new(target.instance_id.clone()).map_err(|_| RequestError::Identity)
}

/// Reads the host file whose bytes are typed at the terminal.
///
/// The size is checked before the read rather than after it, so a caller that named a file larger
/// than one terminal call carries is refused rather than having it read into memory first.
fn read_host_file(path: &std::path::Path) -> Result<Vec<u8>, RequestError> {
    let metadata = std::fs::metadata(path).map_err(|_| RequestError::Content)?;
    if metadata.len() > soma::MAX_PTY_CHUNK_BYTES as u64 {
        return Err(RequestError::ContentTooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| RequestError::Content)?;
    if bytes.len() > soma::MAX_PTY_CHUNK_BYTES {
        // The file grew between the two calls. A shortened write would type something the caller
        // never asked to type, so it is refused instead.
        return Err(RequestError::ContentTooLarge);
    }
    Ok(bytes)
}
