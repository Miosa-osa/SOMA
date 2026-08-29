//! Single-use head ownership ledger.
//!
//! One [`HeadToken`] may own at most one head in its lifetime and one head name may be
//! assigned at most once; a released token is retired forever so a replayed lease can never
//! hand an Instance a head another Instance has written to.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::head::{HeadName, HeadToken};

/// Evidence that a head was assigned to a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseReceipt {
    token: HeadToken,
    name: HeadName,
}

impl LeaseReceipt {
    /// The owning token.
    #[must_use]
    pub fn token(&self) -> HeadToken {
        self.token
    }

    /// The assigned head name.
    #[must_use]
    pub fn name(&self) -> &HeadName {
        &self.name
    }
}

/// Why a lease or release was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaseError {
    /// The token already owns a head.
    TokenAssigned(HeadToken),
    /// The token owned a head before and can never own another.
    TokenRetired(HeadToken),
    /// The head name is already owned or was owned before.
    NameUsed(HeadName),
    /// The token owns nothing.
    UnknownToken(HeadToken),
}

impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenAssigned(_) => f.write_str("token already owns a head"),
            Self::TokenRetired(_) => f.write_str("token is retired"),
            Self::NameUsed(name) => write!(f, "head {name} was already assigned"),
            Self::UnknownToken(_) => f.write_str("token owns no head"),
        }
    }
}

impl std::error::Error for LeaseError {}

/// In-memory ledger of assigned and retired heads for one capability directory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeadLedger {
    assigned: BTreeMap<HeadToken, HeadName>,
    names: BTreeMap<HeadName, HeadToken>,
    retired_tokens: BTreeSet<HeadToken>,
    retired_names: BTreeSet<HeadName>,
}

impl HeadLedger {
    /// An empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns `name` to `token`.
    ///
    /// # Errors
    ///
    /// Refuses a token that owns or owned a head and a name that is or was assigned; a refused
    /// lease changes nothing.
    pub fn lease(&mut self, token: HeadToken, name: HeadName) -> Result<LeaseReceipt, LeaseError> {
        if self.assigned.contains_key(&token) {
            return Err(LeaseError::TokenAssigned(token));
        }
        if self.retired_tokens.contains(&token) {
            return Err(LeaseError::TokenRetired(token));
        }
        if self.names.contains_key(&name) || self.retired_names.contains(&name) {
            return Err(LeaseError::NameUsed(name));
        }
        self.assigned.insert(token, name.clone());
        self.names.insert(name.clone(), token);
        Ok(LeaseReceipt { token, name })
    }

    /// Retires `token` and returns the name it owned.
    ///
    /// # Errors
    ///
    /// Returns [`LeaseError::UnknownToken`] when the token owns nothing.
    pub fn release(&mut self, token: HeadToken) -> Result<HeadName, LeaseError> {
        let name = self
            .assigned
            .remove(&token)
            .ok_or(LeaseError::UnknownToken(token))?;
        self.names.remove(&name);
        self.retired_tokens.insert(token);
        self.retired_names.insert(name.clone());
        Ok(name)
    }

    /// The head currently owned by `token`, if any.
    #[must_use]
    pub fn assigned_name(&self, token: HeadToken) -> Option<&HeadName> {
        self.assigned.get(&token)
    }

    /// The token that currently owns `name`, if any.
    #[must_use]
    pub fn owner(&self, name: &HeadName) -> Option<HeadToken> {
        self.names.get(name).copied()
    }

    /// True when `name` was assigned and later released.
    #[must_use]
    pub fn is_retired_name(&self, name: &HeadName) -> bool {
        self.retired_names.contains(name)
    }

    /// Every current assignment in token order.
    pub fn assignments(&self) -> impl Iterator<Item = (HeadToken, &HeadName)> {
        self.assigned.iter().map(|(token, name)| (*token, name))
    }

    /// Number of current assignments.
    #[must_use]
    pub fn assigned_count(&self) -> usize {
        self.assigned.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(byte: u8) -> HeadToken {
        HeadToken::new([byte; 16]).expect("non-zero")
    }

    #[test]
    fn lease_then_release_retires_both_token_and_name() {
        let mut ledger = HeadLedger::new();
        let name = token(1).head_name();
        let receipt = ledger.lease(token(1), name.clone()).expect("first lease");
        assert_eq!(receipt.token(), token(1));
        assert_eq!(receipt.name(), &name);
        assert_eq!(ledger.assigned_name(token(1)), Some(&name));
        assert_eq!(ledger.owner(&name), Some(token(1)));
        assert_eq!(ledger.assigned_count(), 1);

        assert_eq!(ledger.release(token(1)), Ok(name.clone()));
        assert_eq!(ledger.assigned_count(), 0);
        assert!(ledger.is_retired_name(&name));
        assert_eq!(
            ledger.lease(token(1), token(2).head_name()),
            Err(LeaseError::TokenRetired(token(1)))
        );
        assert_eq!(
            ledger.lease(token(2), name.clone()),
            Err(LeaseError::NameUsed(name))
        );
    }

    #[test]
    fn double_lease_and_unknown_release_are_refused_without_mutation() {
        let mut ledger = HeadLedger::new();
        ledger.lease(token(1), token(1).head_name()).expect("lease");
        let before = ledger.clone();
        assert_eq!(
            ledger.lease(token(1), token(3).head_name()),
            Err(LeaseError::TokenAssigned(token(1)))
        );
        assert_eq!(
            ledger.lease(token(2), token(1).head_name()),
            Err(LeaseError::NameUsed(token(1).head_name()))
        );
        assert_eq!(
            ledger.release(token(9)),
            Err(LeaseError::UnknownToken(token(9)))
        );
        assert_eq!(ledger, before);
    }

    #[test]
    fn assignments_are_ordered_by_token() {
        let mut ledger = HeadLedger::new();
        ledger.lease(token(7), token(7).head_name()).expect("lease");
        ledger.lease(token(2), token(2).head_name()).expect("lease");
        let tokens: Vec<HeadToken> = ledger.assignments().map(|(token, _)| token).collect();
        assert_eq!(tokens, vec![token(2), token(7)]);
    }
}
