use crate::{GuestSessionMaterial, OperationId};
use std::time::Instant;

use super::{
    channel::AuthChannel,
    error::{ControlError, ControlFailureClass, ControlStage},
    io::{ControlIo, FrameReadError, OwnedIo},
};

pub(super) fn connect<I: ControlIo>(
    material: GuestSessionMaterial,
    io: I,
    deadline: Instant,
) -> Result<(AuthChannel<I>, OperationId), ControlError> {
    let mut io = OwnedIo::new(io);
    let Ok(launch_operation) = OperationId::new(*material.binding().operation()) else {
        return Err(fail(&mut io, ControlFailureClass::Protocol));
    };
    let first = match io.read_frame(crate::handshake::MAX_HANDSHAKE_MESSAGE, 1, deadline) {
        Ok(first) => first,
        Err(FrameReadError::Io) => return Err(fail(&mut io, ControlFailureClass::Io)),
        Err(FrameReadError::Length) => {
            return Err(fail(&mut io, ControlFailureClass::Protocol));
        }
    };
    let Ok(pending) = material.start_responder(&first) else {
        return Err(fail(&mut io, ControlFailureClass::Authentication));
    };
    if io.write_all(pending.response(), deadline).is_err() {
        return Err(fail(&mut io, ControlFailureClass::Io));
    }
    let Ok(session) = pending.finish() else {
        return Err(fail(&mut io, ControlFailureClass::Authentication));
    };
    Ok((AuthChannel::new(io, session), launch_operation))
}

fn fail<I: ControlIo>(io: &mut OwnedIo<I>, class: ControlFailureClass) -> ControlError {
    io.poison_once();
    ControlError::new(ControlStage::Handshake, class)
}
