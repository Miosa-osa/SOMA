//! Preparing a live resize.

use soma::InstanceId;

use super::{PreparedOperation, RequestError};
use crate::cli::ResizeArgs;

pub fn prepare(arguments: ResizeArgs) -> Result<PreparedOperation, RequestError> {
    if arguments.cpu.is_none() && arguments.memory_mib.is_none() {
        return Err(RequestError::Shape);
    }
    let instance_id = InstanceId::new(arguments.instance_id).map_err(|_| RequestError::Identity)?;
    Ok(PreparedOperation::Resize {
        instance_id,
        request: soma_local::ResizeRequest {
            cpu_count: arguments.cpu,
            memory_mb: arguments.memory_mib,
        },
    })
}
