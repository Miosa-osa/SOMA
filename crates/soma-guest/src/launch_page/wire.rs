use snow::{params::HashChoice, resolvers::CryptoResolver, resolvers::DefaultResolver};
use zeroize::Zeroizing;

use crate::{Error, InstancePsk, ResponderPrivateKey, SessionBinding, binding::AUTH_PROFILE};

use super::{
    LAUNCH_PAGE_SIZE,
    network::{self, LaunchNetwork},
};

const DOMAIN: &[u8; 16] = b"SOMA-LAUNCH-PAGE";
pub(super) const PAGE_SCHEMA_VERSION: u16 = 3;
const DIGEST_SIZE: usize = 32;
pub(super) const ENTROPY_SIZE: usize = 64;
pub(super) const RESPONDER_SECRET_SIZE: usize = 32;
pub(super) const NETWORK_OFFSET: usize = 16 + 2 + 2 + 32 + 16 + 16 + 32 + 32 + ENTROPY_SIZE;
pub(super) const RESPONDER_OFFSET: usize = NETWORK_OFFSET + network::ENCODED_SIZE;
pub(super) const DIGEST_OFFSET: usize = RESPONDER_OFFSET + RESPONDER_SECRET_SIZE;
pub(super) const ENCODED_SIZE: usize = DIGEST_OFFSET + DIGEST_SIZE;

pub(super) struct DecodedPage {
    pub(super) binding: SessionBinding,
    pub(super) psk: InstancePsk,
    pub(super) entropy: Zeroizing<[u8; ENTROPY_SIZE]>,
    pub(super) network: LaunchNetwork,
    pub(super) responder: ResponderPrivateKey,
}

pub(super) struct PageFields<'a> {
    pub(super) binding: &'a SessionBinding,
    pub(super) psk: &'a [u8; 32],
    pub(super) entropy: &'a [u8; ENTROPY_SIZE],
    pub(super) network: LaunchNetwork,
    pub(super) responder: &'a [u8; RESPONDER_SECRET_SIZE],
}

pub(super) fn encode(page: &mut [u8; LAUNCH_PAGE_SIZE], fields: &PageFields<'_>) {
    let PageFields {
        binding,
        psk,
        entropy,
        network,
        responder,
    } = *fields;
    let mut cursor = 0;
    write(page, &mut cursor, DOMAIN);
    write(page, &mut cursor, &PAGE_SCHEMA_VERSION.to_be_bytes());
    write(page, &mut cursor, &AUTH_PROFILE.to_be_bytes());
    write(page, &mut cursor, binding.generation());
    write(page, &mut cursor, binding.instance());
    write(page, &mut cursor, binding.operation());
    write(page, &mut cursor, binding.launch_nonce());
    write(page, &mut cursor, psk);
    write(page, &mut cursor, entropy);
    debug_assert_eq!(cursor, NETWORK_OFFSET);
    network.encode(&mut page[NETWORK_OFFSET..RESPONDER_OFFSET]);
    page[RESPONDER_OFFSET..DIGEST_OFFSET].copy_from_slice(responder);
    let digest = digest(&page[..DIGEST_OFFSET]);
    page[DIGEST_OFFSET..ENCODED_SIZE].copy_from_slice(&digest);
}

pub(super) fn decode(page: &[u8]) -> Result<DecodedPage, Error> {
    if page.len() != LAUNCH_PAGE_SIZE || page.get(..16) != Some(DOMAIN) {
        return Err(Error::LaunchPageRejected);
    }
    let mut reader = Reader::new(page);
    reader.take(16)?;
    if reader.u16()? != PAGE_SCHEMA_VERSION || reader.u16()? != AUTH_PROFILE {
        return Err(Error::LaunchPageRejected);
    }
    let generation = reader.array()?;
    let instance = reader.array()?;
    let operation = reader.array()?;
    let launch_nonce = reader.array()?;
    let psk = reader.secret_array()?;
    let entropy = reader.secret_array()?;
    let network = LaunchNetwork::decode(reader.take(network::ENCODED_SIZE)?)?;
    let responder = reader.secret_array::<RESPONDER_SECRET_SIZE>()?;
    let stored_digest: [u8; DIGEST_SIZE] = reader.array()?;
    if reader.cursor != ENCODED_SIZE
        || page[ENCODED_SIZE..].iter().any(|byte| *byte != 0)
        || psk.iter().all(|byte| *byte == 0)
        || entropy.iter().all(|byte| *byte == 0)
        || responder.iter().all(|byte| *byte == 0)
        || !constant_time_equal(&stored_digest, &digest(&page[..DIGEST_OFFSET]))
    {
        return Err(Error::LaunchPageRejected);
    }
    let binding = SessionBinding::new(generation, instance, operation, launch_nonce)
        .map_err(|_| Error::LaunchPageRejected)?;
    let psk = InstancePsk::from_zeroizing(instance, psk).map_err(|_| Error::LaunchPageRejected)?;
    let responder =
        ResponderPrivateKey::from_owned(responder).map_err(|_| Error::LaunchPageRejected)?;
    Ok(DecodedPage {
        binding,
        psk,
        entropy,
        network,
        responder,
    })
}

fn digest(covered: &[u8]) -> [u8; DIGEST_SIZE] {
    let mut hash = DefaultResolver
        .resolve_hash(&HashChoice::Blake2s)
        .expect("the fixed suite provides BLAKE2s");
    hash.input(covered);
    let mut output = [0; DIGEST_SIZE];
    hash.result(&mut output);
    output
}

fn constant_time_equal(left: &[u8; DIGEST_SIZE], right: &[u8; DIGEST_SIZE]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

pub(super) struct Reader<'a> {
    source: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    pub(super) const fn new(source: &'a [u8]) -> Self {
        Self { source, cursor: 0 }
    }

    pub(super) fn take(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(Error::LaunchPageRejected)?;
        let value = self
            .source
            .get(self.cursor..end)
            .ok_or(Error::LaunchPageRejected)?;
        self.cursor = end;
        Ok(value)
    }

    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error::LaunchPageRejected)
    }

    pub(super) fn secret_array<const N: usize>(&mut self) -> Result<Zeroizing<[u8; N]>, Error> {
        let bytes = self.take(N)?;
        let mut secret = Zeroizing::new([0; N]);
        secret.copy_from_slice(bytes);
        Ok(secret)
    }

    pub(super) fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.array::<1>()?[0])
    }

    pub(super) fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    pub(super) fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    pub(super) fn finish(self) -> Result<(), Error> {
        if self.cursor != self.source.len() {
            return Err(Error::LaunchPageRejected);
        }
        Ok(())
    }
}

fn write(destination: &mut [u8], cursor: &mut usize, source: &[u8]) {
    let end = *cursor + source.len();
    destination[*cursor..end].copy_from_slice(source);
    *cursor = end;
}
