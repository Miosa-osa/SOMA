use crate::{Error, MAX_RECORD_PAYLOAD};

use super::{HEADER_SIZE, OperationId};

const MAGIC: &[u8; 4] = b"SOMA";
const VERSION: u16 = 1;

#[derive(Clone, Copy, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Kind {
    Prepare = 1,
    Execute = 2,
    Shutdown = 3,
    File = 4,
    Pty = 5,
    RepairComplete = 129,
    Stdout = 130,
    Stderr = 131,
    Terminal = 132,
    ShutdownAck = 133,
    FileOutcome = 134,
    PtyOutcome = 135,
}

impl Kind {
    fn parse(value: u8) -> Result<Self, Error> {
        match value {
            1 => Ok(Self::Prepare),
            2 => Ok(Self::Execute),
            3 => Ok(Self::Shutdown),
            4 => Ok(Self::File),
            5 => Ok(Self::Pty),
            129 => Ok(Self::RepairComplete),
            130 => Ok(Self::Stdout),
            131 => Ok(Self::Stderr),
            132 => Ok(Self::Terminal),
            133 => Ok(Self::ShutdownAck),
            134 => Ok(Self::FileOutcome),
            135 => Ok(Self::PtyOutcome),
            _ => Err(Error::ApplicationMessageRejected),
        }
    }
}

pub(super) struct DecodedFrame<'a> {
    pub(super) kind: Kind,
    pub(super) operation: OperationId,
    pub(super) body: &'a [u8],
}

pub(super) fn encode(kind: Kind, operation: OperationId, body: &[u8]) -> Result<Vec<u8>, Error> {
    if HEADER_SIZE
        .checked_add(body.len())
        .is_none_or(|size| size > MAX_RECORD_PAYLOAD)
    {
        return Err(Error::ApplicationMessageTooLarge);
    }
    let body_length = u16::try_from(body.len()).map_err(|_| Error::ApplicationMessageTooLarge)?;
    let mut encoded = Vec::with_capacity(HEADER_SIZE + body.len());
    encoded.extend_from_slice(MAGIC);
    encoded.extend_from_slice(&VERSION.to_be_bytes());
    encoded.push(kind as u8);
    encoded.push(0);
    encoded.extend_from_slice(&0_u16.to_be_bytes());
    encoded.extend_from_slice(&operation.to_bytes());
    encoded.extend_from_slice(&body_length.to_be_bytes());
    encoded.extend_from_slice(body);
    Ok(encoded)
}

pub(super) fn decode(encoded: &[u8]) -> Result<DecodedFrame<'_>, Error> {
    if encoded.len() < HEADER_SIZE || encoded.len() > MAX_RECORD_PAYLOAD {
        return Err(Error::ApplicationMessageRejected);
    }
    let mut reader = Reader::new(encoded);
    if reader.take(4)? != MAGIC || reader.u16()? != VERSION {
        return Err(Error::ApplicationMessageRejected);
    }
    let kind = Kind::parse(reader.u8()?)?;
    if reader.u8()? != 0 || reader.u16()? != 0 {
        return Err(Error::ApplicationMessageRejected);
    }
    let operation =
        OperationId::new(reader.array()?).map_err(|_| Error::ApplicationMessageRejected)?;
    let body_length = usize::from(reader.u16()?);
    let body = reader.take(body_length)?;
    reader.finish()?;
    Ok(DecodedFrame {
        kind,
        operation,
        body,
    })
}

pub(super) struct Reader<'a> {
    source: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    pub(super) const fn new(source: &'a [u8]) -> Self {
        Self { source, cursor: 0 }
    }

    pub(super) fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(Error::ApplicationMessageRejected)?;
        let value = self
            .source
            .get(self.cursor..end)
            .ok_or(Error::ApplicationMessageRejected)?;
        self.cursor = end;
        Ok(value)
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error::ApplicationMessageRejected)
    }

    pub(super) fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }

    pub(super) fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(super) fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub(super) fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.array()?))
    }

    pub(super) fn field(&mut self, maximum: usize) -> Result<&'a [u8], Error> {
        let length = usize::from(self.u16()?);
        if length > maximum {
            return Err(Error::ApplicationMessageRejected);
        }
        self.take(length)
    }

    pub(super) fn finish(self) -> Result<(), Error> {
        if self.cursor != self.source.len() {
            return Err(Error::ApplicationMessageRejected);
        }
        Ok(())
    }
}
