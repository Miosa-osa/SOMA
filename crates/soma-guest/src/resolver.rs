use snow::{
    Builder, Error as SnowError,
    params::{CipherChoice, DHChoice, HashChoice, NoiseParams},
    resolvers::{CryptoResolver, DefaultResolver},
    types::{Cipher, Dh, Hash, Random},
};

use crate::Error;

const PUBLIC_KEY_VALIDATION_SCALAR: [u8; 32] = [0xA5; 32];

#[derive(Default)]
struct ContributoryResolver(DefaultResolver);

struct ContributoryDh {
    inner: Box<dyn Dh>,
}

pub(crate) fn noise_builder<'a>(params: NoiseParams) -> Builder<'a> {
    Builder::with_resolver(params, Box::<ContributoryResolver>::default())
}

pub(crate) fn validate_responder_public_key(public: &[u8; 32]) -> Result<(), Error> {
    let resolver = ContributoryResolver::default();
    let mut dh = resolver
        .resolve_dh(&DHChoice::Curve25519)
        .ok_or(Error::CryptoSetup)?;
    dh.set(&PUBLIC_KEY_VALIDATION_SCALAR);
    let mut shared = [0_u8; 32];
    dh.dh(public, &mut shared)
        .map_err(|_| Error::NonContributoryPublicKey)
}

pub(crate) fn fill_os_random(destination: &mut [u8]) -> Result<(), Error> {
    let resolver = DefaultResolver;
    let mut random = resolver.resolve_rng().ok_or(Error::RandomnessUnavailable)?;
    random
        .try_fill_bytes(destination)
        .map_err(|_| Error::RandomnessUnavailable)
}

impl CryptoResolver for ContributoryResolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        self.0.resolve_rng()
    }

    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        self.0
            .resolve_dh(choice)
            .map(|inner| Box::new(ContributoryDh { inner }) as Box<dyn Dh>)
    }

    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        self.0.resolve_hash(choice)
    }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        self.0.resolve_cipher(choice)
    }
}

impl Dh for ContributoryDh {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn pub_len(&self) -> usize {
        self.inner.pub_len()
    }

    fn priv_len(&self) -> usize {
        self.inner.priv_len()
    }

    fn set(&mut self, private: &[u8]) {
        self.inner.set(private);
    }

    fn generate(&mut self, random: &mut dyn Random) -> Result<(), SnowError> {
        self.inner.generate(random)
    }

    fn pubkey(&self) -> &[u8] {
        self.inner.pubkey()
    }

    fn privkey(&self) -> &[u8] {
        self.inner.privkey()
    }

    fn dh(&self, public: &[u8], output: &mut [u8]) -> Result<(), SnowError> {
        self.inner.dh(public, output)?;
        let shared = output.get(..self.inner.dh_len()).ok_or(SnowError::Dh)?;
        if shared.iter().all(|byte| *byte == 0) {
            return Err(SnowError::Dh);
        }
        Ok(())
    }

    fn dh_len(&self) -> usize {
        self.inner.dh_len()
    }
}

#[cfg(test)]
mod tests;
