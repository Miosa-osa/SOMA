//! One encoded section: its role, payload, digest and the sequence decoder.

use super::{
    Digest, FLAG_CRITICAL, MAX_SECTION_BYTES, MAX_SECTIONS, Reader, SECTION_VERSION, SectionError,
    SectionRole, Writer,
};

/// One known, bounded, digest-covered section.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    role: SectionRole,
    payload: Vec<u8>,
}

impl Section {
    /// Wraps a payload whose length is within [`MAX_SECTION_BYTES`].
    ///
    /// # Errors
    ///
    /// Returns [`SectionError::PayloadTooLarge`] when the payload exceeds the bound.
    pub fn new(role: SectionRole, payload: Vec<u8>) -> Result<Self, SectionError> {
        let length = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        if length > u64::from(MAX_SECTION_BYTES) {
            return Err(SectionError::PayloadTooLarge { length });
        }
        Ok(Self { role, payload })
    }

    #[must_use]
    pub const fn role(&self) -> SectionRole {
        self.role
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    #[must_use]
    pub fn digest(&self) -> Digest {
        Digest::of(&self.payload)
    }

    pub(crate) fn encode(&self, writer: &mut Writer) {
        writer.put_u16(self.role.code());
        writer.put_u16(SECTION_VERSION);
        writer.put_u8(FLAG_CRITICAL);
        // `new` bounded the length to `MAX_SECTION_BYTES`, which fits in `u32`.
        writer.put_u32(u32::try_from(self.payload.len()).unwrap_or(MAX_SECTION_BYTES));
        writer.put_bytes(self.digest().as_bytes());
        writer.put_bytes(&self.payload);
    }

    /// Decodes `count` sections in strictly ascending role order.
    ///
    /// # Errors
    ///
    /// Returns a typed [`SectionError`] for any malformed, duplicated, reordered, unknown
    /// critical, unsupported, or digest-mismatched section.
    pub(crate) fn decode_sequence(
        reader: &mut Reader<'_>,
        count: u16,
    ) -> Result<Vec<Self>, SectionError> {
        let mut sections = Vec::with_capacity(usize::from(count.min(MAX_SECTIONS)));
        let mut previous: Option<u16> = None;
        for _ in 0..count {
            let code = reader.u16()?;
            if let Some(previous) = previous
                && code <= previous
            {
                return Err(SectionError::RoleOrder {
                    previous,
                    next: code,
                });
            }
            previous = Some(code);
            if let Some(section) = Self::decode_body(reader, code)? {
                sections.push(section);
            }
        }
        Ok(sections)
    }

    fn decode_body(reader: &mut Reader<'_>, code: u16) -> Result<Option<Self>, SectionError> {
        let version = reader.u16()?;
        let flags = reader.u8()?;
        if flags & !FLAG_CRITICAL != 0 {
            return Err(SectionError::ReservedFlags(flags));
        }
        let critical = flags & FLAG_CRITICAL != 0;
        let role = SectionRole::from_code(code);
        match role {
            Some(_) if version != SECTION_VERSION => {
                return Err(SectionError::UnsupportedVersion {
                    role: code,
                    version,
                });
            }
            Some(role) if !critical => return Err(SectionError::KnownRoleNotCritical(role)),
            None if critical => return Err(SectionError::UnknownCriticalRole(code)),
            _ => {}
        }
        let length = reader.length_u32(MAX_SECTION_BYTES)?;
        let expected = Digest::from_bytes(reader.array()?);
        let payload = reader.take(length)?;
        if Digest::of(payload) != expected {
            return Err(SectionError::DigestMismatch { role: code });
        }
        Ok(role.map(|role| Self {
            role,
            payload: payload.to_vec(),
        }))
    }
}
