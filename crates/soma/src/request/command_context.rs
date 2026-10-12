//! What a command runs with, beside its program and arguments.
//!
//! The bounds are the guest protocol's own (`soma-guest`'s `CommandContext`), repeated here
//! because this crate does not depend on the guest codec. A test in `soma-api`, which depends on
//! both, pins the two to each other so they cannot drift.

use std::fmt;

use super::ValidationError;

/// The most environment pairs one command may carry.
pub const MAX_ENVIRONMENT: usize = 64;
/// The largest environment name, environment value, or working directory.
pub const MAX_FIELD_BYTES: usize = 4_096;
/// The largest user name.
pub const MAX_USER_BYTES: usize = 256;
/// The most standard input one command may be given.
pub const MAX_STDIN_BYTES: usize = 32 * 1024;

/// The environment, working directory, user, and standard input of one command.
///
/// An empty context is the default: the agent's fixed environment, its default directory and
/// account, and a closed standard input. A command with an empty context is encoded and
/// fingerprinted exactly as it was before contexts existed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CommandContext {
    environment: Vec<(String, String)>,
    working_directory: Option<String>,
    user: Option<String>,
    stdin: Vec<u8>,
}

impl CommandContext {
    /// Creates one bounded context.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::InvalidCommand`] for too many pairs, an empty name or a name
    /// holding `=`, a field over its bound or holding NUL, a relative or empty working
    /// directory, an empty or oversized user, or oversized standard input.
    pub fn new(
        environment: Vec<(String, String)>,
        working_directory: Option<String>,
        user: Option<String>,
        stdin: Vec<u8>,
    ) -> Result<Self, ValidationError> {
        let names_ok = environment.iter().all(|(name, value)| {
            !name.is_empty()
                && !name.contains('=')
                && name.len() <= MAX_FIELD_BYTES
                && value.len() <= MAX_FIELD_BYTES
                && !name.contains('\0')
                && !value.contains('\0')
        });
        let directory_ok = working_directory.as_deref().is_none_or(|path| {
            path.starts_with('/') && path.len() <= MAX_FIELD_BYTES && !path.contains('\0')
        });
        let user_ok = user.as_deref().is_none_or(|name| {
            !name.is_empty() && name.len() <= MAX_USER_BYTES && !name.contains('\0')
        });
        if environment.len() > MAX_ENVIRONMENT
            || !names_ok
            || !directory_ok
            || !user_ok
            || stdin.len() > MAX_STDIN_BYTES
        {
            return Err(ValidationError::InvalidCommand);
        }
        Ok(Self {
            environment,
            working_directory,
            user,
            stdin,
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.environment.is_empty()
            && self.working_directory.is_none()
            && self.user.is_none()
            && self.stdin.is_empty()
    }

    #[must_use]
    pub fn environment(&self) -> &[(String, String)] {
        &self.environment
    }

    #[must_use]
    pub fn working_directory(&self) -> Option<&str> {
        self.working_directory.as_deref()
    }

    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    #[must_use]
    pub fn stdin(&self) -> &[u8] {
        &self.stdin
    }

    /// The bytes this context adds to a guest command body, measured the way the guest codec
    /// measures them: two length bytes per field, plus the standard input as it is.
    pub(super) fn encoded_size(&self) -> usize {
        let field = |bytes: usize| 2 + bytes;
        self.environment
            .iter()
            .map(|(name, value)| field(name.len()) + field(value.len()))
            .sum::<usize>()
            + self
                .working_directory
                .as_deref()
                .map_or(0, |path| field(path.len()))
            + self.user.as_deref().map_or(0, |user| field(user.len()))
            + self.stdin.len()
    }
}

impl fmt::Debug for CommandContext {
    /// Reports shapes and lengths only: an environment value or standard input is tenant data
    /// and a credential is the ordinary case.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CommandContext")
            .field("environment_count", &self.environment.len())
            .field(
                "working_directory_bytes",
                &self.working_directory.as_ref().map(String::len),
            )
            .field("user_bytes", &self.user.as_ref().map(String::len))
            .field("stdin_bytes", &self.stdin.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn the_default_context_is_empty_and_unmeasured() {
        let context = CommandContext::default();
        assert!(context.is_empty());
        assert_eq!(context.encoded_size(), 0);
    }

    #[test]
    fn a_full_context_is_measured_like_the_guest_codec() {
        let context = CommandContext::new(
            vec![pair("A", "1"), pair("BB", "")],
            Some("/work".to_owned()),
            Some("dev".to_owned()),
            b"in".to_vec(),
        )
        .expect("context");
        // (2+1)+(2+1) + (2+2)+(2+0) + (2+5) + (2+3) + 2
        assert_eq!(context.encoded_size(), 3 + 3 + 4 + 2 + 7 + 5 + 2);
        assert!(!context.is_empty());
    }

    #[test]
    fn every_bound_is_enforced() {
        let too_many = (0..=MAX_ENVIRONMENT)
            .map(|i| pair(&format!("K{i}"), "v"))
            .collect();
        let long = "x".repeat(MAX_FIELD_BYTES + 1);
        let cases: Vec<Result<CommandContext, ValidationError>> = vec![
            CommandContext::new(too_many, None, None, vec![]),
            CommandContext::new(vec![pair("", "v")], None, None, vec![]),
            CommandContext::new(vec![pair("A=B", "v")], None, None, vec![]),
            CommandContext::new(vec![pair("A\0", "v")], None, None, vec![]),
            CommandContext::new(vec![pair("A", "v\0")], None, None, vec![]),
            CommandContext::new(vec![pair(&long, "v")], None, None, vec![]),
            CommandContext::new(vec![pair("A", &long)], None, None, vec![]),
            CommandContext::new(vec![], Some("relative".to_owned()), None, vec![]),
            CommandContext::new(vec![], Some(String::new()), None, vec![]),
            CommandContext::new(vec![], Some(format!("/{long}")), None, vec![]),
            CommandContext::new(vec![], None, Some(String::new()), vec![]),
            CommandContext::new(vec![], None, Some("u".repeat(MAX_USER_BYTES + 1)), vec![]),
            CommandContext::new(vec![], None, None, vec![0; MAX_STDIN_BYTES + 1]),
        ];
        for case in cases {
            assert_eq!(case, Err(ValidationError::InvalidCommand));
        }
        assert!(CommandContext::new(vec![], None, None, vec![0; MAX_STDIN_BYTES]).is_ok());
    }

    #[test]
    fn debug_never_prints_values() {
        let context = CommandContext::new(
            vec![pair("TOKEN", "hunter2")],
            None,
            None,
            b"secret".to_vec(),
        )
        .expect("context");
        let shown = format!("{context:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("TOKEN") && !shown.contains("secret")
        );
    }
}
