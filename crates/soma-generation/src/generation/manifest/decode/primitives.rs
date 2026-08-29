use super::{Decoder, MAX_SHORT_STRING, invalid, limit};
use crate::generation::{
    artifacts::{ArtifactDescriptor, ArtifactRole, Sha256Digest},
    contracts::ContractBinding,
    error::CompileError,
};

impl<'a> Decoder<'a> {
    pub(super) fn tag(&mut self, expected: u8) -> Result<(), CompileError> {
        if self.u8()? != expected {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) fn contract(&mut self, tag: u8) -> Result<ContractBinding, CompileError> {
        self.tag(tag)?;
        Ok(ContractBinding {
            version: self.u16()?,
            digest: self.digest()?,
        })
    }

    pub(super) fn descriptor(
        &mut self,
        expected: ArtifactRole,
    ) -> Result<ArtifactDescriptor, CompileError> {
        let role = ArtifactRole::from_code(self.u8()?).ok_or_else(invalid)?;
        let media_type = self.short_string()?;
        if role != expected || media_type != role.media_type() {
            return Err(invalid());
        }
        let digest = self.digest()?;
        let size = self.u64()?;
        if self.seen.contains(&digest) {
            return Err(invalid());
        }
        self.seen.push(digest);
        Ok(ArtifactDescriptor { role, digest, size })
    }

    pub(super) fn optional_digest(&mut self) -> Result<Option<Sha256Digest>, CompileError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.digest()?)),
            _ => Err(invalid()),
        }
    }

    pub(super) fn optional_string(&mut self) -> Result<Option<String>, CompileError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.short_string()?)),
            _ => Err(invalid()),
        }
    }

    pub(super) fn short_string(&mut self) -> Result<String, CompileError> {
        let length = usize::from(self.u16()?);
        if length > MAX_SHORT_STRING {
            return Err(limit());
        }
        let value = self.consume(length)?;
        if value.contains(&0) {
            return Err(invalid());
        }
        String::from_utf8(value.to_vec()).map_err(|_| invalid())
    }

    pub(super) fn digest(&mut self) -> Result<Sha256Digest, CompileError> {
        Ok(Sha256Digest::from_bytes(self.array()?))
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], CompileError> {
        self.consume(N)?.try_into().map_err(|_| invalid())
    }

    pub(super) fn consume(&mut self, count: usize) -> Result<&'a [u8], CompileError> {
        let end = self.offset.checked_add(count).ok_or_else(invalid)?;
        let value = self.bytes.get(self.offset..end).ok_or_else(invalid)?;
        self.offset = end;
        Ok(value)
    }

    pub(super) fn u8(&mut self) -> Result<u8, CompileError> {
        Ok(self.consume(1)?[0])
    }

    pub(super) fn u16(&mut self) -> Result<u16, CompileError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64, CompileError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
}
