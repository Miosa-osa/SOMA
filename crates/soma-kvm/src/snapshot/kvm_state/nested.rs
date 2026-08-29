//! Optional nested-virtualization state with an explicit VMX or SVM format.
//!
//! Machine contract v1 does not support nested virtualization, so a certified v1 snapshot
//! is expected to carry `None`; the encoding exists so presence is explicit rather than
//! silently defaulted.

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

/// Size of one VMCS12 or VMCB12 blob.
pub const NESTED_BLOB_LEN: usize = 4096;

const FORMAT_VMX: u8 = 0;
const FORMAT_SVM: u8 = 1;

#[derive(Clone, Eq, PartialEq)]
pub enum NestedState {
    Vmx {
        flags: u16,
        vmxon_pa: u64,
        vmcs12_pa: u64,
        smm_flags: u16,
        hdr_flags: u32,
        preemption_timer_deadline: u64,
        vmcs12: Box<[u8; NESTED_BLOB_LEN]>,
        shadow_vmcs12: Box<[u8; NESTED_BLOB_LEN]>,
    },
    Svm {
        flags: u16,
        vmcb_pa: u64,
        vmcb12: Box<[u8; NESTED_BLOB_LEN]>,
    },
}

impl std::fmt::Debug for NestedState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vmx { flags, .. } => write!(formatter, "NestedState::Vmx(flags={flags:#x})"),
            Self::Svm { flags, .. } => write!(formatter, "NestedState::Svm(flags={flags:#x})"),
        }
    }
}

impl NestedState {
    #[must_use]
    pub const fn flags(&self) -> u16 {
        match self {
            Self::Vmx { flags, .. } | Self::Svm { flags, .. } => *flags,
        }
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        match self {
            Self::Vmx {
                flags,
                vmxon_pa,
                vmcs12_pa,
                smm_flags,
                hdr_flags,
                preemption_timer_deadline,
                vmcs12,
                shadow_vmcs12,
            } => {
                writer.put_u8(FORMAT_VMX);
                writer.put_u16(*flags);
                writer.put_u64(*vmxon_pa);
                writer.put_u64(*vmcs12_pa);
                writer.put_u16(*smm_flags);
                writer.put_u32(*hdr_flags);
                writer.put_u64(*preemption_timer_deadline);
                writer.put_bytes(vmcs12.as_slice());
                writer.put_bytes(shadow_vmcs12.as_slice());
            }
            Self::Svm {
                flags,
                vmcb_pa,
                vmcb12,
            } => {
                writer.put_u8(FORMAT_SVM);
                writer.put_u16(*flags);
                writer.put_u64(*vmcb_pa);
                writer.put_bytes(vmcb12.as_slice());
            }
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let format = reader.u8()?;
        match format {
            FORMAT_VMX => {
                let flags = reader.u16()?;
                let vmxon_pa = reader.u64()?;
                let vmcs12_pa = reader.u64()?;
                let smm_flags = reader.u16()?;
                let hdr_flags = reader.u32()?;
                let preemption_timer_deadline = reader.u64()?;
                let vmcs12 = read_blob(reader)?;
                let shadow_vmcs12 = read_blob(reader)?;
                Ok(Self::Vmx {
                    flags,
                    vmxon_pa,
                    vmcs12_pa,
                    smm_flags,
                    hdr_flags,
                    preemption_timer_deadline,
                    vmcs12,
                    shadow_vmcs12,
                })
            }
            FORMAT_SVM => Ok(Self::Svm {
                flags: reader.u16()?,
                vmcb_pa: reader.u64()?,
                vmcb12: read_blob(reader)?,
            }),
            other => Err(KvmStateError::UnknownCode {
                field: "nested.format",
                code: u32::from(other),
            }),
        }
    }
}

fn read_blob(reader: &mut Reader<'_>) -> Result<Box<[u8; NESTED_BLOB_LEN]>, KvmStateError> {
    let bytes = reader.take(NESTED_BLOB_LEN)?;
    let mut blob = Box::new([0_u8; NESTED_BLOB_LEN]);
    blob.copy_from_slice(bytes);
    Ok(blob)
}
