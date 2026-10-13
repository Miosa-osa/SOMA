//! The volume request frames of ADR 0053 Step A.
//!
//! Both frames are fixed width, so a shortened or lengthened frame is refused rather than read
//! as a different request. They live apart from the rest of the request codec to hold that file
//! inside the source budget.

use crate::volume::MAX_TENANT_BYTES;
use crate::{GenerationId, InstanceId, OperationId, RequestFingerprint, VolumeId};

use super::super::{ProtocolError, array};

/// The fixed length of one volume attach frame.
const ATTACH_FRAME_LEN: usize = 1 + 16 + 16 + 16 + 32 + 4 + 32 + 2 + MAX_TENANT_BYTES;
/// The fixed length of one volume detach frame.
const DETACH_FRAME_LEN: usize = 1 + 16 + 16 + 16 + 32;

/// One volume attach: bind the volume to an Instance, creating it on the first attach.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachFrame {
    /// The operation, which is the idempotency key.
    pub operation: OperationId,
    /// The owning Instance.
    pub instance: InstanceId,
    /// The volume.
    pub volume: VolumeId,
    /// The fingerprint of this request under the operation.
    pub fingerprint: RequestFingerprint,
    /// The requested disk class in GiB.
    pub size_class_gib: u32,
    /// The Generation the owning Instance was launched from.
    pub generation: GenerationId,
    /// The owning tenant label.
    pub tenant: String,
}

impl AttachFrame {
    pub(super) fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.operation.as_bytes());
        out.extend_from_slice(self.instance.as_bytes());
        out.extend_from_slice(self.volume.as_bytes());
        out.extend_from_slice(self.fingerprint.as_bytes());
        out.extend_from_slice(&self.size_class_gib.to_be_bytes());
        out.extend_from_slice(self.generation.as_bytes());
        let tenant = self.tenant.as_bytes();
        // The label field is fixed width, so a label past the bound is cut here rather than
        // shifting the fields after it, and the decoder refuses such a frame on the padding.
        let taken = tenant.len().min(MAX_TENANT_BYTES);
        let mut field = [0_u8; MAX_TENANT_BYTES];
        field[..taken].copy_from_slice(&tenant[..taken]);
        out.extend_from_slice(&u16::try_from(taken).map_or([0, 0], u16::to_be_bytes));
        out.extend_from_slice(&field);
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != ATTACH_FRAME_LEN {
            return Err(ProtocolError("volume attach"));
        }
        let length_at = ATTACH_FRAME_LEN - MAX_TENANT_BYTES - 2;
        let tenant_len = usize::from(u16::from_be_bytes(array(&bytes[length_at..length_at + 2])));
        let field = &bytes[ATTACH_FRAME_LEN - MAX_TENANT_BYTES..];
        if tenant_len == 0
            || tenant_len > MAX_TENANT_BYTES
            || field[tenant_len..].iter().any(|byte| *byte != 0)
        {
            return Err(ProtocolError("tenant"));
        }
        let tenant = std::str::from_utf8(&field[..tenant_len])
            .map_err(|_| ProtocolError("tenant"))?
            .to_owned();
        Ok(Self {
            operation: OperationId::new(array(&bytes[1..17]))
                .map_err(|_| ProtocolError("operation"))?,
            instance: InstanceId::new(array(&bytes[17..33]))
                .map_err(|_| ProtocolError("instance"))?,
            volume: VolumeId::new(array(&bytes[33..49])).map_err(|_| ProtocolError("volume"))?,
            fingerprint: RequestFingerprint::new(array(&bytes[49..81]))
                .map_err(|_| ProtocolError("fingerprint"))?,
            size_class_gib: u32::from_be_bytes(array(&bytes[81..85])),
            generation: GenerationId::new(array(&bytes[85..117]))
                .map_err(|_| ProtocolError("generation"))?,
            tenant,
        })
    }
}

/// One volume detach: release the volume from its Instance, keeping its file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetachFrame {
    /// The operation, which is the idempotency key.
    pub operation: OperationId,
    /// The owning Instance.
    pub instance: InstanceId,
    /// The volume.
    pub volume: VolumeId,
    /// The fingerprint of this request under the operation.
    pub fingerprint: RequestFingerprint,
}

impl DetachFrame {
    pub(super) fn encode_into(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(self.operation.as_bytes());
        out.extend_from_slice(self.instance.as_bytes());
        out.extend_from_slice(self.volume.as_bytes());
        out.extend_from_slice(self.fingerprint.as_bytes());
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != DETACH_FRAME_LEN {
            return Err(ProtocolError("volume detach"));
        }
        Ok(Self {
            operation: OperationId::new(array(&bytes[1..17]))
                .map_err(|_| ProtocolError("operation"))?,
            instance: InstanceId::new(array(&bytes[17..33]))
                .map_err(|_| ProtocolError("instance"))?,
            volume: VolumeId::new(array(&bytes[33..49])).map_err(|_| ProtocolError("volume"))?,
            fingerprint: RequestFingerprint::new(array(&bytes[49..81]))
                .map_err(|_| ProtocolError("fingerprint"))?,
        })
    }
}
