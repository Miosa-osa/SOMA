use crate::{DeliveredHostLaunchMaterial, OperationId};

use super::{
    channel::AuthChannel,
    deadline,
    error::{ControlError, ControlFailureClass, ControlStage},
    io::{FrameReadError, HostControlIo, OwnedIo},
};

pub(super) fn connect<I: HostControlIo>(
    material: DeliveredHostLaunchMaterial,
    io: I,
) -> Result<(AuthChannel<I>, OperationId), ControlError> {
    let mut io = OwnedIo::new(io);
    let deadline = deadline::handshake();
    let Ok(launch_operation) = OperationId::new(*material.binding().operation()) else {
        return Err(fail(&mut io, ControlFailureClass::Protocol));
    };
    let Ok((waiting, first)) = material.start_initiator() else {
        return Err(fail(&mut io, ControlFailureClass::Authentication));
    };
    if io.write_all(&first, deadline).is_err() {
        return Err(fail(&mut io, ControlFailureClass::Io));
    }
    let second = match io.read_frame(crate::handshake::MAX_HANDSHAKE_MESSAGE, 1, deadline) {
        Ok(second) => second,
        Err(FrameReadError::Io) => return Err(fail(&mut io, ControlFailureClass::Io)),
        Err(FrameReadError::Length) => {
            return Err(fail(&mut io, ControlFailureClass::Protocol));
        }
    };
    let Ok(session) = waiting.finish(&second) else {
        return Err(fail(&mut io, ControlFailureClass::Authentication));
    };
    Ok((AuthChannel::new(io, session), launch_operation))
}

fn fail<I: HostControlIo>(io: &mut OwnedIo<I>, class: ControlFailureClass) -> ControlError {
    io.poison_once();
    ControlError::new(ControlStage::Handshake, class)
}
