use crate::{
    AuthenticatedSession, ControlIo, GuestMessage, HostMessage, record::MAX_RECORD_CIPHERTEXT,
    record::MIN_RECORD_CIPHERTEXT,
};

use super::{
    error::{ControlError, ControlFailureClass, ControlStage},
    io::{FrameReadError, OwnedIo},
};

pub(crate) struct AuthChannel<I: ControlIo> {
    pub(crate) io: OwnedIo<I>,
    pub(crate) session: AuthenticatedSession,
}

impl<I: ControlIo> AuthChannel<I> {
    pub(crate) const fn new(io: OwnedIo<I>, session: AuthenticatedSession) -> Self {
        Self { io, session }
    }

    pub(crate) fn send_host(
        &mut self,
        message: &HostMessage,
        deadline: Instant,
    ) -> Result<(), ChannelFailure> {
        let encoded = message.encode().map_err(|_| ChannelFailure::Protocol)?;
        self.send(&encoded, deadline)
    }

    pub(crate) fn send_guest(
        &mut self,
        message: &GuestMessage,
        deadline: Instant,
    ) -> Result<(), ChannelFailure> {
        let encoded = message.encode().map_err(|_| ChannelFailure::Protocol)?;
        self.send(&encoded, deadline)
    }

    pub(crate) fn receive_host(
        &mut self,
        deadline: Instant,
    ) -> Result<HostMessage, ChannelFailure> {
        let payload = self.receive(deadline)?;
        HostMessage::decode(&payload).map_err(|_| ChannelFailure::Protocol)
    }

    pub(crate) fn receive_guest(
        &mut self,
        deadline: Instant,
    ) -> Result<GuestMessage, ChannelFailure> {
        let payload = self.receive(deadline)?;
        GuestMessage::decode(&payload).map_err(|_| ChannelFailure::Protocol)
    }

    pub(crate) fn fail(&mut self, stage: ControlStage, class: ControlFailureClass) -> ControlError {
        self.io.poison_once();
        ControlError::new(stage, class)
    }

    fn send(&mut self, payload: &[u8], deadline: Instant) -> Result<(), ChannelFailure> {
        let record = self
            .session
            .seal(payload)
            .map_err(|_| ChannelFailure::Protocol)?;
        self.io
            .write_all(&record, deadline)
            .map_err(|()| ChannelFailure::Io)
    }

    fn receive(&mut self, deadline: Instant) -> Result<Vec<u8>, ChannelFailure> {
        let record =
            match self
                .io
                .read_frame(MAX_RECORD_CIPHERTEXT, MIN_RECORD_CIPHERTEXT, deadline)
            {
                Ok(record) => record,
                Err(FrameReadError::Io) => return Err(ChannelFailure::Io),
                Err(FrameReadError::Length) => return Err(ChannelFailure::Protocol),
            };
        self.session
            .open(&record)
            .map_err(|_| ChannelFailure::Authentication)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ChannelFailure {
    Io,
    Authentication,
    Protocol,
}

impl ChannelFailure {
    pub(crate) const fn class(self) -> ControlFailureClass {
        match self {
            Self::Io => ControlFailureClass::Io,
            Self::Authentication => ControlFailureClass::Authentication,
            Self::Protocol => ControlFailureClass::Protocol,
        }
    }
}

pub(crate) fn channel_failure<I: ControlIo>(
    channel: &mut AuthChannel<I>,
    stage: ControlStage,
    failure: ChannelFailure,
) -> ControlError {
    channel.fail(stage, failure.class())
}
use std::time::Instant;
