//! GSI routing table (`KVM_SET_GSI_ROUTING`) with typed route targets.

use super::KvmStateError;
use crate::snapshot::wire::{Reader, Writer};

pub const MAX_IRQ_ROUTES: u16 = 256;

const TARGET_IRQCHIP: u8 = 1;
const TARGET_MSI: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteTarget {
    Irqchip {
        irqchip: u32,
        pin: u32,
    },
    Msi {
        address_lo: u32,
        address_hi: u32,
        data: u32,
        devid: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IrqRoutingEntry {
    pub gsi: u32,
    pub flags: u32,
    pub target: RouteTarget,
}

impl IrqRoutingEntry {
    fn write(&self, writer: &mut Writer) {
        writer.put_u32(self.gsi);
        writer.put_u32(self.flags);
        match self.target {
            RouteTarget::Irqchip { irqchip, pin } => {
                writer.put_u8(TARGET_IRQCHIP);
                writer.put_u32(irqchip);
                writer.put_u32(pin);
            }
            RouteTarget::Msi {
                address_lo,
                address_hi,
                data,
                devid,
            } => {
                writer.put_u8(TARGET_MSI);
                writer.put_u32(address_lo);
                writer.put_u32(address_hi);
                writer.put_u32(data);
                writer.put_u32(devid);
            }
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let gsi = reader.u32()?;
        let flags = reader.u32()?;
        let target = match reader.u8()? {
            TARGET_IRQCHIP => RouteTarget::Irqchip {
                irqchip: reader.u32()?,
                pin: reader.u32()?,
            },
            TARGET_MSI => RouteTarget::Msi {
                address_lo: reader.u32()?,
                address_hi: reader.u32()?,
                data: reader.u32()?,
                devid: reader.u32()?,
            },
            other => {
                return Err(KvmStateError::UnknownCode {
                    field: "irq_routing.target",
                    code: u32::from(other),
                });
            }
        };
        Ok(Self { gsi, flags, target })
    }
}

/// Complete `IrqRouting` section payload: a bounded table with unique `(gsi, target)` rows.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IrqRoutingState {
    entries: Vec<IrqRoutingEntry>,
}

impl IrqRoutingState {
    /// # Errors
    ///
    /// Returns [`KvmStateError::TooManyEntries`] or [`KvmStateError::DuplicateEntry`].
    pub fn new(entries: Vec<IrqRoutingEntry>) -> Result<Self, KvmStateError> {
        if entries.len() > usize::from(MAX_IRQ_ROUTES) {
            return Err(KvmStateError::TooManyEntries {
                field: "irq_routing",
                count: entries.len(),
            });
        }
        for (position, entry) in entries.iter().enumerate() {
            if entries[..position].iter().any(|e| e == entry) {
                return Err(KvmStateError::DuplicateEntry {
                    field: "irq_routing",
                    key: u64::from(entry.gsi),
                });
            }
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[IrqRoutingEntry] {
        &self.entries
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(2 + self.entries.len() * 25);
        writer.put_u16(u16::try_from(self.entries.len()).unwrap_or(MAX_IRQ_ROUTES));
        for entry in &self.entries {
            entry.write(&mut writer);
        }
        writer.finish()
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError`] for short, oversized, unknown-target, duplicate, or
    /// trailing input.
    pub fn decode(bytes: &[u8]) -> Result<Self, KvmStateError> {
        let mut reader = Reader::new(bytes);
        let count = reader.count_u16(MAX_IRQ_ROUTES)?;
        let mut entries = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            entries.push(IrqRoutingEntry::read(&mut reader)?);
        }
        reader.finish()?;
        Self::new(entries)
    }
}
