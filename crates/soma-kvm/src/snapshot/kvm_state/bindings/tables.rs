//! CPUID and MSR table conversions through the `kvm-bindings` FAM wrappers.

use kvm_bindings::{CpuId, Msrs, kvm_cpuid_entry2, kvm_msr_entry};

use super::BindingError;
use crate::snapshot::kvm_state::{CpuidEntries, CpuidEntry, MsrEntries, MsrEntry};

impl TryFrom<&CpuId> for CpuidEntries {
    type Error = BindingError;

    fn try_from(cpuid: &CpuId) -> Result<Self, BindingError> {
        let entries = cpuid
            .as_slice()
            .iter()
            .map(|entry| CpuidEntry {
                function: entry.function,
                index: entry.index,
                flags: entry.flags,
                eax: entry.eax,
                ebx: entry.ebx,
                ecx: entry.ecx,
                edx: entry.edx,
            })
            .collect();
        Ok(Self::new(entries)?)
    }
}

impl TryFrom<&CpuidEntries> for CpuId {
    type Error = BindingError;

    fn try_from(entries: &CpuidEntries) -> Result<Self, BindingError> {
        let raw: Vec<kvm_cpuid_entry2> = entries
            .entries()
            .iter()
            .map(|entry| kvm_cpuid_entry2 {
                function: entry.function,
                index: entry.index,
                flags: entry.flags,
                eax: entry.eax,
                ebx: entry.ebx,
                ecx: entry.ecx,
                edx: entry.edx,
                padding: [0; 3],
            })
            .collect();
        Self::from_entries(&raw).map_err(|_| BindingError::TableTooLarge {
            field: "cpuid",
            count: raw.len(),
        })
    }
}

impl TryFrom<&Msrs> for MsrEntries {
    type Error = BindingError;

    fn try_from(msrs: &Msrs) -> Result<Self, BindingError> {
        let entries = msrs
            .as_slice()
            .iter()
            .map(|entry| MsrEntry {
                index: entry.index,
                value: entry.data,
            })
            .collect();
        Ok(Self::new(entries)?)
    }
}

impl TryFrom<&MsrEntries> for Msrs {
    type Error = BindingError;

    fn try_from(entries: &MsrEntries) -> Result<Self, BindingError> {
        let raw: Vec<kvm_msr_entry> = entries
            .entries()
            .iter()
            .map(|entry| kvm_msr_entry {
                index: entry.index,
                reserved: 0,
                data: entry.value,
            })
            .collect();
        Self::from_entries(&raw).map_err(|_| BindingError::TableTooLarge {
            field: "msrs",
            count: raw.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use kvm_bindings::{CpuId, Msrs};

    use crate::snapshot::kvm_state::{CpuidEntries, CpuidEntry, MsrEntries, MsrEntry};

    #[test]
    fn cpuid_and_msr_tables_round_trip() {
        let cpuid = CpuidEntries::new(vec![
            CpuidEntry {
                function: 0,
                eax: 0xd,
                ..CpuidEntry::default()
            },
            CpuidEntry {
                function: 1,
                ecx: 1 << 31,
                ..CpuidEntry::default()
            },
        ])
        .unwrap();
        let raw = CpuId::try_from(&cpuid).unwrap();
        assert_eq!(raw.as_slice().len(), 2);
        assert_eq!(CpuidEntries::try_from(&raw).unwrap(), cpuid);

        let msrs = MsrEntries::new(vec![MsrEntry {
            index: 0x10,
            value: 42,
        }])
        .unwrap();
        let raw = Msrs::try_from(&msrs).unwrap();
        assert_eq!(raw.as_slice()[0].data, 42);
        assert_eq!(MsrEntries::try_from(&raw).unwrap(), msrs);
    }
}
