use zeroize::Zeroizing;

use crate::{Error, InstancePsk, SessionBinding, binding::AUTH_PROFILE};

use super::LAUNCH_PAGE_SIZE;

const DOMAIN: &[u8; 16] = b"SOMA-LAUNCH-PAGE";
const PAGE_SCHEMA_VERSION: u16 = 1;
pub(super) const ENTROPY_SIZE: usize = 64;
pub(super) const ENCODED_SIZE: usize = 16 + 2 + 2 + 32 + 16 + 16 + 32 + 32 + ENTROPY_SIZE;

pub(super) struct DecodedPage {
    pub(super) binding: SessionBinding,
    pub(super) psk: InstancePsk,
    pub(super) entropy: Zeroizing<[u8; ENTROPY_SIZE]>,
}

pub(super) fn encode(
    page: &mut [u8; LAUNCH_PAGE_SIZE],
    binding: &SessionBinding,
    psk: &[u8; 32],
    entropy: &[u8; ENTROPY_SIZE],
) {
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
    debug_assert_eq!(cursor, ENCODED_SIZE);
}

pub(super) fn decode(page: &[u8]) -> Result<DecodedPage, Error> {
    if page.len() != LAUNCH_PAGE_SIZE || page.get(..16) != Some(DOMAIN) {
        return Err(Error::LaunchPageRejected);
    }
    let mut cursor = 16;
    if read_u16(page, &mut cursor)? != PAGE_SCHEMA_VERSION
        || read_u16(page, &mut cursor)? != AUTH_PROFILE
    {
        return Err(Error::LaunchPageRejected);
    }
    let generation = array(page, advance(&mut cursor, 32)?)?;
    let instance = array(page, advance(&mut cursor, 16)?)?;
    let operation = array(page, advance(&mut cursor, 16)?)?;
    let launch_nonce = array(page, advance(&mut cursor, 32)?)?;
    let psk = secret_array(page, advance(&mut cursor, 32)?)?;
    let entropy = secret_array(page, advance(&mut cursor, ENTROPY_SIZE)?)?;
    if cursor != ENCODED_SIZE
        || page[cursor..].iter().any(|byte| *byte != 0)
        || psk.iter().all(|byte| *byte == 0)
        || entropy.iter().all(|byte| *byte == 0)
    {
        return Err(Error::LaunchPageRejected);
    }
    let binding = SessionBinding::new(generation, instance, operation, launch_nonce)
        .map_err(|_| Error::LaunchPageRejected)?;
    let psk = InstancePsk::from_zeroizing(instance, psk).map_err(|_| Error::LaunchPageRejected)?;
    Ok(DecodedPage {
        binding,
        psk,
        entropy,
    })
}

fn array<const N: usize>(source: &[u8], start: usize) -> Result<[u8; N], Error> {
    source
        .get(start..start.checked_add(N).ok_or(Error::LaunchPageRejected)?)
        .ok_or(Error::LaunchPageRejected)?
        .try_into()
        .map_err(|_| Error::LaunchPageRejected)
}

fn secret_array<const N: usize>(source: &[u8], start: usize) -> Result<Zeroizing<[u8; N]>, Error> {
    let bytes = source
        .get(start..start.checked_add(N).ok_or(Error::LaunchPageRejected)?)
        .ok_or(Error::LaunchPageRejected)?;
    let mut secret = Zeroizing::new([0; N]);
    secret.copy_from_slice(bytes);
    Ok(secret)
}

fn advance(cursor: &mut usize, amount: usize) -> Result<usize, Error> {
    let start = *cursor;
    *cursor = cursor
        .checked_add(amount)
        .ok_or(Error::LaunchPageRejected)?;
    Ok(start)
}

fn read_u16(source: &[u8], cursor: &mut usize) -> Result<u16, Error> {
    let start = advance(cursor, 2)?;
    Ok(u16::from_be_bytes(array(source, start)?))
}

fn write(destination: &mut [u8], cursor: &mut usize, source: &[u8]) {
    let end = *cursor + source.len();
    destination[*cursor..end].copy_from_slice(source);
    *cursor = end;
}
