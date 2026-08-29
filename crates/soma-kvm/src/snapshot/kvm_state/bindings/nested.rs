//! Nested-state conversions through `kvm_ioctls::KvmNestedStateBuffer`.
//!
//! Reading the KVM header and data unions is the only unsafe operation, and every union
//! member is plain integer or byte-array data for which all bit patterns are valid.

use kvm_bindings::{
    KVM_STATE_NESTED_FORMAT_SVM, KVM_STATE_NESTED_FORMAT_VMX, kvm_nested_state__bindgen_ty_1,
    kvm_svm_nested_state_hdr, kvm_vmx_nested_state_hdr, kvm_vmx_nested_state_hdr__bindgen_ty_1,
    nested::{
        KvmNestedStateBuffer, kvm_nested_state__data, kvm_svm_nested_state_data,
        kvm_vmx_nested_state_data,
    },
};

use super::BindingError;
use crate::snapshot::kvm_state::NestedState;

impl TryFrom<&KvmNestedStateBuffer> for NestedState {
    type Error = BindingError;

    fn try_from(buffer: &KvmNestedStateBuffer) -> Result<Self, BindingError> {
        match u32::from(buffer.format) {
            KVM_STATE_NESTED_FORMAT_VMX => {
                // SAFETY: `hdr` and `data` are C unions filled by KVM (or zeroed by
                // `KvmNestedStateBuffer::empty`). The `vmx` members are plain integers and
                // byte arrays, so reading them for the VMX format is always defined.
                let (hdr, data) = unsafe { (buffer.hdr.vmx, buffer.data.vmx) };
                Ok(Self::Vmx {
                    flags: buffer.flags,
                    vmxon_pa: hdr.vmxon_pa,
                    vmcs12_pa: hdr.vmcs12_pa,
                    smm_flags: hdr.smm.flags,
                    hdr_flags: hdr.flags,
                    preemption_timer_deadline: hdr.preemption_timer_deadline,
                    vmcs12: Box::new(data.vmcs12),
                    shadow_vmcs12: Box::new(data.shadow_vmcs12),
                })
            }
            KVM_STATE_NESTED_FORMAT_SVM => {
                // SAFETY: as above; the `svm` members are one integer and one byte array.
                let (hdr, data) = unsafe { (buffer.hdr.svm, buffer.data.svm) };
                Ok(Self::Svm {
                    flags: buffer.flags,
                    vmcb_pa: hdr.vmcb_pa,
                    vmcb12: Box::new(data.vmcb12),
                })
            }
            _ => Err(BindingError::UnsupportedNestedFormat(buffer.format)),
        }
    }
}

impl From<&NestedState> for KvmNestedStateBuffer {
    fn from(state: &NestedState) -> Self {
        let mut buffer = Self::empty();
        buffer.flags = state.flags();
        match state {
            NestedState::Vmx {
                vmxon_pa,
                vmcs12_pa,
                smm_flags,
                hdr_flags,
                preemption_timer_deadline,
                vmcs12,
                shadow_vmcs12,
                ..
            } => {
                buffer.format = u16::try_from(KVM_STATE_NESTED_FORMAT_VMX).unwrap_or(0);
                buffer.hdr = kvm_nested_state__bindgen_ty_1 {
                    vmx: kvm_vmx_nested_state_hdr {
                        vmxon_pa: *vmxon_pa,
                        vmcs12_pa: *vmcs12_pa,
                        smm: kvm_vmx_nested_state_hdr__bindgen_ty_1 { flags: *smm_flags },
                        pad: 0,
                        flags: *hdr_flags,
                        preemption_timer_deadline: *preemption_timer_deadline,
                    },
                };
                buffer.data = kvm_nested_state__data {
                    vmx: kvm_vmx_nested_state_data {
                        vmcs12: **vmcs12,
                        shadow_vmcs12: **shadow_vmcs12,
                    },
                };
            }
            NestedState::Svm {
                vmcb_pa, vmcb12, ..
            } => {
                buffer.format = u16::try_from(KVM_STATE_NESTED_FORMAT_SVM).unwrap_or(1);
                buffer.hdr = kvm_nested_state__bindgen_ty_1 {
                    svm: kvm_svm_nested_state_hdr { vmcb_pa: *vmcb_pa },
                };
                buffer.data = kvm_nested_state__data {
                    svm: kvm_svm_nested_state_data { vmcb12: **vmcb12 },
                };
            }
        }
        buffer
    }
}

#[cfg(test)]
mod tests {
    use kvm_bindings::nested::KvmNestedStateBuffer;

    use super::{BindingError, NestedState};
    use crate::snapshot::kvm_state::NESTED_BLOB_LEN;

    #[test]
    fn vmx_and_svm_round_trip_and_unknown_format_is_rejected() {
        let mut vmcs12 = Box::new([0_u8; NESTED_BLOB_LEN]);
        vmcs12[0] = 0xaa;
        let vmx = NestedState::Vmx {
            flags: 1,
            vmxon_pa: 0x1000,
            vmcs12_pa: 0x2000,
            smm_flags: 0,
            hdr_flags: 4,
            preemption_timer_deadline: 9,
            vmcs12,
            shadow_vmcs12: Box::new([0; NESTED_BLOB_LEN]),
        };
        let buffer = KvmNestedStateBuffer::from(&vmx);
        assert_eq!(NestedState::try_from(&buffer).unwrap(), vmx);

        let svm = NestedState::Svm {
            flags: 2,
            vmcb_pa: 0x3000,
            vmcb12: Box::new([7; NESTED_BLOB_LEN]),
        };
        let mut buffer = KvmNestedStateBuffer::from(&svm);
        assert_eq!(NestedState::try_from(&buffer).unwrap(), svm);
        buffer.format = 9;
        assert_eq!(
            NestedState::try_from(&buffer),
            Err(BindingError::UnsupportedNestedFormat(9))
        );
    }
}
