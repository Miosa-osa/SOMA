use std::io::{self, Read};

use tar::EntryType;

use super::{
    ExtensionPolicy, MAX_LOCAL_PAX_RECORD_BYTES, PreflightBudget, PreflightError, preflight,
};

mod bounds;
mod naming;

const RAW_HEADER_CEILING: u32 = 1_000_000;
const EXTENSION_BYTE_CEILING: u64 = 64 * 1024 * 1024;
const IMPORT_POLICY: ExtensionPolicy = ExtensionPolicy {
    long_record_ceiling: 4_097,
    pax_record_ceiling: MAX_LOCAL_PAX_RECORD_BYTES,
};

fn run_preflight(
    reader: impl Read,
    maximum: u64,
    policy: ExtensionPolicy,
) -> Result<(), PreflightError> {
    let mut budget = PreflightBudget::new(RAW_HEADER_CEILING, EXTENSION_BYTE_CEILING);
    preflight(reader, maximum, policy, &mut budget)
}

fn append_pax(builder: &mut tar::Builder<&mut Vec<u8>>, key: &str) {
    builder
        .append_pax_extensions([(key, b"pax-value".as_slice())])
        .unwrap();
}

fn append_gnu(builder: &mut tar::Builder<&mut Vec<u8>>, kind: EntryType) {
    let body = b"gnu-value\0";
    let mut header = tar::Header::new_gnu();
    header.set_path("extension").unwrap();
    header.set_entry_type(kind);
    header.set_size(body.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append(&header, body.as_slice()).unwrap();
}

fn header(kind: EntryType, size: u64) -> [u8; 512] {
    let mut header = tar::Header::new_gnu();
    header.set_path("extension").unwrap();
    header.set_entry_type(kind);
    header.set_size(size);
    header.set_mode(0o644);
    header.set_cksum();
    *header.as_bytes()
}

struct HeaderOnly {
    header: Box<[u8; 512]>,
    position: usize,
}

impl HeaderOnly {
    fn new(header: &[u8; 512]) -> Self {
        Self {
            header: Box::new(*header),
            position: 0,
        }
    }
}

impl Read for HeaderOnly {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(self.position < self.header.len(), "extension body was read");
        let count = output.len().min(self.header.len() - self.position);
        output[..count].copy_from_slice(&self.header[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

struct PrefixOnly<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> PrefixOnly<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }
}

impl Read for PrefixOnly<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(
            self.position < self.bytes.len(),
            "read beyond bounded headers"
        );
        let count = output.len().min(self.bytes.len() - self.position);
        output[..count].copy_from_slice(&self.bytes[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}
