//! In-kernel interrupt-controller state: master and slave PIC plus the IOAPIC.

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

pub const IOAPIC_PINS: usize = 24;

/// One 8259 PIC as reported by `KVM_GET_IRQCHIP`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PicState {
    pub last_irr: u8,
    pub irr: u8,
    pub imr: u8,
    pub isr: u8,
    pub priority_add: u8,
    pub irq_base: u8,
    pub read_reg_select: u8,
    pub poll: u8,
    pub special_mask: u8,
    pub init_state: u8,
    pub auto_eoi: u8,
    pub rotate_on_auto_eoi: u8,
    pub special_fully_nested_mode: u8,
    pub init4: u8,
    pub elcr: u8,
    pub elcr_mask: u8,
}

impl PicState {
    pub const ENCODED_LEN: usize = 16;

    const fn as_array(&self) -> [u8; 16] {
        [
            self.last_irr,
            self.irr,
            self.imr,
            self.isr,
            self.priority_add,
            self.irq_base,
            self.read_reg_select,
            self.poll,
            self.special_mask,
            self.init_state,
            self.auto_eoi,
            self.rotate_on_auto_eoi,
            self.special_fully_nested_mode,
            self.init4,
            self.elcr,
            self.elcr_mask,
        ]
    }

    #[must_use]
    pub const fn from_array(bytes: [u8; 16]) -> Self {
        let [
            last_irr,
            irr,
            imr,
            isr,
            priority_add,
            irq_base,
            read_reg_select,
            poll,
            special_mask,
            init_state,
            auto_eoi,
            rotate_on_auto_eoi,
            special_fully_nested_mode,
            init4,
            elcr,
            elcr_mask,
        ] = bytes;
        Self {
            last_irr,
            irr,
            imr,
            isr,
            priority_add,
            irq_base,
            read_reg_select,
            poll,
            special_mask,
            init_state,
            auto_eoi,
            rotate_on_auto_eoi,
            special_fully_nested_mode,
            init4,
            elcr,
            elcr_mask,
        }
    }

    fn write(&self, writer: &mut Writer) {
        writer.put_bytes(&self.as_array());
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        Ok(Self::from_array(reader.array()?))
    }
}

/// IOAPIC registers with every redirection entry carried as its raw 64-bit value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IoapicState {
    pub base_address: u64,
    pub ioregsel: u32,
    pub id: u32,
    pub irr: u32,
    pub redirection: [u64; IOAPIC_PINS],
}

impl Default for IoapicState {
    fn default() -> Self {
        Self {
            base_address: 0,
            ioregsel: 0,
            id: 0,
            irr: 0,
            redirection: [0; IOAPIC_PINS],
        }
    }
}

impl IoapicState {
    pub const ENCODED_LEN: usize = 8 + 4 + 4 + 4 + IOAPIC_PINS * 8;

    fn write(&self, writer: &mut Writer) {
        writer.put_u64(self.base_address);
        writer.put_u32(self.ioregsel);
        writer.put_u32(self.id);
        writer.put_u32(self.irr);
        for entry in self.redirection {
            writer.put_u64(entry);
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let base_address = reader.u64()?;
        let ioregsel = reader.u32()?;
        let id = reader.u32()?;
        let irr = reader.u32()?;
        let mut redirection = [0_u64; IOAPIC_PINS];
        for entry in &mut redirection {
            *entry = reader.u64()?;
        }
        Ok(Self {
            base_address,
            ioregsel,
            id,
            irr,
            redirection,
        })
    }
}

/// Complete `Irqchip` section payload.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IrqchipState {
    pub master: PicState,
    pub slave: PicState,
    pub ioapic: IoapicState,
}

impl IrqchipState {
    pub const ENCODED_LEN: usize = 2 * PicState::ENCODED_LEN + IoapicState::ENCODED_LEN;

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(Self::ENCODED_LEN);
        self.master.write(&mut writer);
        self.slave.write(&mut writer);
        self.ioapic.write(&mut writer);
        writer.finish()
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError::Wire`] for short or trailing input.
    pub fn decode(bytes: &[u8]) -> Result<Self, KvmStateError> {
        let mut reader = Reader::new(bytes);
        let state = Self {
            master: PicState::read(&mut reader)?,
            slave: PicState::read(&mut reader)?,
            ioapic: IoapicState::read(&mut reader)?,
        };
        reader.finish()?;
        Ok(state)
    }
}
