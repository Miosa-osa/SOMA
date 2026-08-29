//! PIC, IOAPIC, and GSI-routing conversions.
//!
//! Reading the IOAPIC redirection and routing unions is the only unsafe operation, and
//! every union member is plain integer data for which all bit patterns are valid.

use kvm_bindings::{
    KVM_IRQ_ROUTING_IRQCHIP, KVM_IRQ_ROUTING_MSI, kvm_ioapic_state, kvm_ioapic_state__bindgen_ty_1,
    kvm_irq_routing_entry, kvm_irq_routing_entry__bindgen_ty_1, kvm_irq_routing_irqchip,
    kvm_irq_routing_msi, kvm_irq_routing_msi__bindgen_ty_1, kvm_pic_state,
};

use super::BindingError;
use crate::snapshot::kvm_state::{IoapicState, IrqRoutingEntry, PicState, RouteTarget};

impl From<kvm_pic_state> for PicState {
    fn from(pic: kvm_pic_state) -> Self {
        Self::from_array([
            pic.last_irr,
            pic.irr,
            pic.imr,
            pic.isr,
            pic.priority_add,
            pic.irq_base,
            pic.read_reg_select,
            pic.poll,
            pic.special_mask,
            pic.init_state,
            pic.auto_eoi,
            pic.rotate_on_auto_eoi,
            pic.special_fully_nested_mode,
            pic.init4,
            pic.elcr,
            pic.elcr_mask,
        ])
    }
}

impl From<PicState> for kvm_pic_state {
    fn from(pic: PicState) -> Self {
        Self {
            last_irr: pic.last_irr,
            irr: pic.irr,
            imr: pic.imr,
            isr: pic.isr,
            priority_add: pic.priority_add,
            irq_base: pic.irq_base,
            read_reg_select: pic.read_reg_select,
            poll: pic.poll,
            special_mask: pic.special_mask,
            init_state: pic.init_state,
            auto_eoi: pic.auto_eoi,
            rotate_on_auto_eoi: pic.rotate_on_auto_eoi,
            special_fully_nested_mode: pic.special_fully_nested_mode,
            init4: pic.init4,
            elcr: pic.elcr,
            elcr_mask: pic.elcr_mask,
        }
    }
}

impl From<&kvm_ioapic_state> for IoapicState {
    fn from(ioapic: &kvm_ioapic_state) -> Self {
        let mut redirection = [0_u64; 24];
        for (value, entry) in redirection.iter_mut().zip(ioapic.redirtbl) {
            // SAFETY: the redirection entry is a union of `bits: u64` and a bitfield struct
            // of the same size; reading the integer view is defined for every bit pattern.
            *value = unsafe { entry.bits };
        }
        Self {
            base_address: ioapic.base_address,
            ioregsel: ioapic.ioregsel,
            id: ioapic.id,
            irr: ioapic.irr,
            redirection,
        }
    }
}

impl From<&IoapicState> for kvm_ioapic_state {
    fn from(ioapic: &IoapicState) -> Self {
        let mut raw = Self {
            base_address: ioapic.base_address,
            ioregsel: ioapic.ioregsel,
            id: ioapic.id,
            irr: ioapic.irr,
            ..Self::default()
        };
        for (entry, bits) in raw.redirtbl.iter_mut().zip(ioapic.redirection) {
            *entry = kvm_ioapic_state__bindgen_ty_1 { bits };
        }
        raw
    }
}

impl TryFrom<&kvm_irq_routing_entry> for IrqRoutingEntry {
    type Error = BindingError;

    fn try_from(entry: &kvm_irq_routing_entry) -> Result<Self, BindingError> {
        let target = match entry.type_ {
            KVM_IRQ_ROUTING_IRQCHIP => {
                // SAFETY: `u` is a union of plain-integer structs padded to a common size;
                // reading the `irqchip` view is defined for every bit pattern.
                let irqchip = unsafe { entry.u.irqchip };
                RouteTarget::Irqchip {
                    irqchip: irqchip.irqchip,
                    pin: irqchip.pin,
                }
            }
            KVM_IRQ_ROUTING_MSI => {
                // SAFETY: as above for the `msi` view and its nested `devid` union.
                let msi = unsafe { entry.u.msi };
                let devid = unsafe { msi.__bindgen_anon_1.devid };
                RouteTarget::Msi {
                    address_lo: msi.address_lo,
                    address_hi: msi.address_hi,
                    data: msi.data,
                    devid,
                }
            }
            other => return Err(BindingError::UnsupportedRouteType(other)),
        };
        Ok(Self {
            gsi: entry.gsi,
            flags: entry.flags,
            target,
        })
    }
}

impl From<IrqRoutingEntry> for kvm_irq_routing_entry {
    fn from(entry: IrqRoutingEntry) -> Self {
        let (type_, u) = match entry.target {
            RouteTarget::Irqchip { irqchip, pin } => (
                KVM_IRQ_ROUTING_IRQCHIP,
                kvm_irq_routing_entry__bindgen_ty_1 {
                    irqchip: kvm_irq_routing_irqchip { irqchip, pin },
                },
            ),
            RouteTarget::Msi {
                address_lo,
                address_hi,
                data,
                devid,
            } => (
                KVM_IRQ_ROUTING_MSI,
                kvm_irq_routing_entry__bindgen_ty_1 {
                    msi: kvm_irq_routing_msi {
                        address_lo,
                        address_hi,
                        data,
                        __bindgen_anon_1: kvm_irq_routing_msi__bindgen_ty_1 { devid },
                    },
                },
            ),
        };
        Self {
            gsi: entry.gsi,
            type_,
            flags: entry.flags,
            pad: 0,
            u,
        }
    }
}

#[cfg(test)]
mod tests {
    use kvm_bindings::{kvm_ioapic_state, kvm_irq_routing_entry, kvm_pic_state};

    use super::super::BindingError;
    use crate::snapshot::kvm_state::{IoapicState, IrqRoutingEntry, PicState, RouteTarget};

    #[test]
    fn pic_ioapic_and_routing_round_trip() {
        let pic = PicState::from_array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(PicState::from(kvm_pic_state::from(pic)), pic);

        let mut ioapic = IoapicState::default();
        ioapic.redirection[5] = 0x1_0000_0025;
        let raw = kvm_ioapic_state::from(&ioapic);
        assert_eq!(IoapicState::from(&raw), ioapic);

        let route = IrqRoutingEntry {
            gsi: 5,
            flags: 0,
            target: RouteTarget::Irqchip { irqchip: 2, pin: 5 },
        };
        let raw = kvm_irq_routing_entry::from(route);
        assert_eq!(IrqRoutingEntry::try_from(&raw), Ok(route));
        let msi = IrqRoutingEntry {
            gsi: 30,
            flags: 0,
            target: RouteTarget::Msi {
                address_lo: 1,
                address_hi: 2,
                data: 3,
                devid: 4,
            },
        };
        let raw = kvm_irq_routing_entry::from(msi);
        assert_eq!(IrqRoutingEntry::try_from(&raw), Ok(msi));
        let mut unsupported = raw;
        unsupported.type_ = 4;
        assert_eq!(
            IrqRoutingEntry::try_from(&unsupported),
            Err(BindingError::UnsupportedRouteType(4))
        );
    }
}
