use crate::Error;

/// An authenticated action on the guest's declared workspace device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WorkspaceAction {
    /// Unmount the volume and take exclusive ownership of the now-offline device.
    Unmount = 0,
    /// Mount the currently assigned backing and bind its persistent directories.
    Mount = 1,
}

impl WorkspaceAction {
    pub(super) fn decode(body: &[u8]) -> Result<Self, Error> {
        match body {
            [0] => Ok(Self::Unmount),
            [1] => Ok(Self::Mount),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

/// The result of changing the guest workspace mount state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WorkspaceOutcome {
    /// The requested mount state is now in effect.
    Ready = 0,
    /// The request was refused and the previous state is intact.
    Refused = 1,
    /// The agent could not restore the previous mount state.
    Inconsistent = 2,
    /// This guest or Generation does not offer workspace mount transitions.
    Unsupported = 3,
}

impl WorkspaceOutcome {
    pub(super) fn decode(body: &[u8]) -> Result<Self, Error> {
        match body {
            [0] => Ok(Self::Ready),
            [1] => Ok(Self::Refused),
            [2] => Ok(Self::Inconsistent),
            [3] => Ok(Self::Unsupported),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}
