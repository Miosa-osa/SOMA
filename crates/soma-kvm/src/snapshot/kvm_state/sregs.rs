//! Special register encodings: the eight segment registers with their cached descriptors, the
//! two descriptor table registers, and the control registers around them.
//!
//! These are separated from the general-purpose registers because they are the half that can be
//! invalid. A general-purpose register is eighteen u64 values with no wrong value among them; a
//! segment descriptor carries a type nibble and a privilege level that a snapshot could name out
//! of range, so every read here validates before it returns.

use super::{KvmStateError, invalid};
use crate::snapshot::wire::{Reader, Writer};

/// One segment register with its cached descriptor.
///
/// The boolean fields mirror the architectural descriptor bits one to one, so they stay as
/// booleans rather than being folded into an opaque bitmask.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Segment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    /// Descriptor type nibble (0..=15).
    pub type_: u8,
    pub present: bool,
    /// Descriptor privilege level (0..=3).
    pub dpl: u8,
    pub db: bool,
    pub s: bool,
    pub l: bool,
    pub g: bool,
    pub avl: bool,
    pub unusable: bool,
}

impl Segment {
    pub const ENCODED_LEN: usize = 8 + 4 + 2 + 1 + 1 + 1 + 6;

    /// # Errors
    ///
    /// Returns [`KvmStateError::InvalidField`] when `type_` or `dpl` is out of range.
    pub fn validate(&self) -> Result<(), KvmStateError> {
        if self.type_ > 0x0f {
            return Err(invalid("segment.type", self.type_));
        }
        if self.dpl > 3 {
            return Err(invalid("segment.dpl", self.dpl));
        }
        Ok(())
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_u64(self.base);
        writer.put_u32(self.limit);
        writer.put_u16(self.selector);
        writer.put_u8(self.type_);
        writer.put_presence(self.present);
        writer.put_u8(self.dpl);
        for flag in [self.db, self.s, self.l, self.g, self.avl, self.unusable] {
            writer.put_presence(flag);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let segment = Self {
            base: reader.u64()?,
            limit: reader.u32()?,
            selector: reader.u16()?,
            type_: reader.u8()?,
            present: reader.presence()?,
            dpl: reader.u8()?,
            db: reader.presence()?,
            s: reader.presence()?,
            l: reader.presence()?,
            g: reader.presence()?,
            avl: reader.presence()?,
            unusable: reader.presence()?,
        };
        segment.validate()?;
        Ok(segment)
    }
}

/// GDT or IDT descriptor table register.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Dtable {
    pub base: u64,
    pub limit: u16,
}

impl Dtable {
    pub(crate) fn write(&self, writer: &mut Writer) {
        writer.put_u64(self.base);
        writer.put_u16(self.limit);
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        Ok(Self {
            base: reader.u64()?,
            limit: reader.u16()?,
        })
    }
}

/// Special registers: segments, tables, control registers, EFER, APIC base, interrupt bitmap.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sregs {
    pub cs: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
    pub ss: Segment,
    pub tr: Segment,
    pub ldt: Segment,
    pub gdt: Dtable,
    pub idt: Dtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

impl Sregs {
    pub const ENCODED_LEN: usize = 8 * Segment::ENCODED_LEN + 2 * 10 + 7 * 8 + 4 * 8;

    const fn segments(&self) -> [Segment; 8] {
        [
            self.cs, self.ds, self.es, self.fs, self.gs, self.ss, self.tr, self.ldt,
        ]
    }

    /// # Errors
    ///
    /// Returns [`KvmStateError::InvalidField`] when any segment is out of range.
    pub fn validate(&self) -> Result<(), KvmStateError> {
        self.segments().iter().try_for_each(Segment::validate)
    }

    pub(crate) fn write(&self, writer: &mut Writer) {
        for segment in self.segments() {
            segment.write(writer);
        }
        self.gdt.write(writer);
        self.idt.write(writer);
        for value in [
            self.cr0,
            self.cr2,
            self.cr3,
            self.cr4,
            self.cr8,
            self.efer,
            self.apic_base,
        ] {
            writer.put_u64(value);
        }
        for value in self.interrupt_bitmap {
            writer.put_u64(value);
        }
    }

    pub(crate) fn read(reader: &mut Reader<'_>) -> Result<Self, KvmStateError> {
        let mut segments = [Segment::default(); 8];
        for segment in &mut segments {
            *segment = Segment::read(reader)?;
        }
        let [cs, ds, es, fs, gs, ss, tr, ldt] = segments;
        let gdt = Dtable::read(reader)?;
        let idt = Dtable::read(reader)?;
        let mut controls = [0_u64; 7];
        for value in &mut controls {
            *value = reader.u64()?;
        }
        let [cr0, cr2, cr3, cr4, cr8, efer, apic_base] = controls;
        let mut interrupt_bitmap = [0_u64; 4];
        for value in &mut interrupt_bitmap {
            *value = reader.u64()?;
        }
        Ok(Self {
            cs,
            ds,
            es,
            fs,
            gs,
            ss,
            tr,
            ldt,
            gdt,
            idt,
            cr0,
            cr2,
            cr3,
            cr4,
            cr8,
            efer,
            apic_base,
            interrupt_bitmap,
        })
    }
}
