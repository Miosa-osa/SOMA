//! Typed failures for parsing, resolution infrastructure, and lock decoding.

use std::{error::Error, fmt};

use crate::{rejection::Rejection, wire::WireError};

/// A bound or shape violation on one named field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundError {
    Empty { field: String },
    TooLong { field: String, maximum: usize },
    TooMany { field: String, maximum: usize },
    ForbiddenCharacter { field: String },
    InvalidShape { field: String },
}

impl BoundError {
    #[must_use]
    pub fn field(&self) -> &str {
        match self {
            Self::Empty { field }
            | Self::TooLong { field, .. }
            | Self::TooMany { field, .. }
            | Self::ForbiddenCharacter { field }
            | Self::InvalidShape { field } => field,
        }
    }
}

impl fmt::Display for BoundError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { field } => write!(formatter, "field `{field}` must not be empty"),
            Self::TooLong { field, maximum } => {
                write!(formatter, "field `{field}` exceeds {maximum} bytes")
            }
            Self::TooMany { field, maximum } => {
                write!(formatter, "field `{field}` exceeds {maximum} entries")
            }
            Self::ForbiddenCharacter { field } => {
                write!(formatter, "field `{field}` contains a forbidden character")
            }
            Self::InvalidShape { field } => {
                write!(formatter, "field `{field}` has an invalid shape")
            }
        }
    }
}

impl Error for BoundError {}

/// Why a Template document was not accepted as a `soma.template/v1alpha1` document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    Oversized {
        length: usize,
        maximum: usize,
    },
    NotUtf8,
    Syntax(String),
    UnsupportedSchema {
        found: String,
    },
    MissingField {
        field: String,
    },
    UnknownField {
        field: String,
    },
    WrongType {
        field: String,
        expected: &'static str,
    },
    InvalidValue {
        field: String,
        reason: String,
    },
    Bound(BoundError),
}

impl ParseError {
    /// The document field the failure names, when one applies.
    #[must_use]
    pub fn field(&self) -> Option<&str> {
        match self {
            Self::MissingField { field }
            | Self::UnknownField { field }
            | Self::WrongType { field, .. }
            | Self::InvalidValue { field, .. } => Some(field),
            Self::Bound(bound) => Some(bound.field()),
            Self::Oversized { .. }
            | Self::NotUtf8
            | Self::Syntax(_)
            | Self::UnsupportedSchema { .. } => None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversized { length, maximum } => {
                write!(
                    formatter,
                    "document has {length} bytes, maximum is {maximum}"
                )
            }
            Self::NotUtf8 => formatter.write_str("document is not valid UTF-8"),
            Self::Syntax(message) => write!(formatter, "TOML syntax: {message}"),
            Self::UnsupportedSchema { found } => {
                write!(formatter, "unsupported template schema `{found}`")
            }
            Self::MissingField { field } => write!(formatter, "missing field `{field}`"),
            Self::UnknownField { field } => write!(formatter, "unknown field `{field}`"),
            Self::WrongType { field, expected } => {
                write!(formatter, "field `{field}` must be {expected}")
            }
            Self::InvalidValue { field, reason } => {
                write!(formatter, "field `{field}` is invalid: {reason}")
            }
            Self::Bound(bound) => bound.fmt(formatter),
        }
    }
}

impl Error for ParseError {}

impl From<BoundError> for ParseError {
    fn from(bound: BoundError) -> Self {
        Self::Bound(bound)
    }
}

/// An external collaborator that could not answer, as opposed to a Template that was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExternalDependency {
    OciResolver,
    FilesystemOracle,
}

impl fmt::Display for ExternalDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OciResolver => "OCI resolver",
            Self::FilesystemOracle => "filesystem oracle",
        })
    }
}

/// Why resolution did not produce a Template Lock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TemplateError {
    /// The Template violated one of the required validation classes.
    Rejected(Rejection),
    /// A resolver or oracle failed for a reason unrelated to the Template's content.
    Unavailable {
        dependency: ExternalDependency,
        detail: String,
    },
}

impl TemplateError {
    /// The rejection, when the failure was a Template-content decision.
    #[must_use]
    pub const fn rejection(&self) -> Option<&Rejection> {
        match self {
            Self::Rejected(rejection) => Some(rejection),
            Self::Unavailable { .. } => None,
        }
    }
}

impl fmt::Display for TemplateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(rejection) => rejection.fmt(formatter),
            Self::Unavailable { dependency, detail } => {
                write!(formatter, "{dependency} unavailable: {detail}")
            }
        }
    }
}

impl Error for TemplateError {}

impl From<Rejection> for TemplateError {
    fn from(rejection: Rejection) -> Self {
        Self::Rejected(rejection)
    }
}

/// Why lock bytes were not accepted as a canonical Template Lock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LockError {
    BadMagic,
    UnsupportedLockSchema(u16),
    UnsupportedTemplateSchema(String),
    Wire(WireError),
    InvalidDiscriminant { field: &'static str, value: u8 },
    InvalidField { field: &'static str },
}

impl fmt::Display for LockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => formatter.write_str("missing SOMALOCK magic"),
            Self::UnsupportedLockSchema(version) => {
                write!(formatter, "unsupported lock schema {version}")
            }
            Self::UnsupportedTemplateSchema(schema) => {
                write!(formatter, "unsupported template schema `{schema}`")
            }
            Self::Wire(wire) => wire.fmt(formatter),
            Self::InvalidDiscriminant { field, value } => {
                write!(
                    formatter,
                    "field `{field}` has invalid discriminant {value}"
                )
            }
            Self::InvalidField { field } => write!(formatter, "field `{field}` is invalid"),
        }
    }
}

impl Error for LockError {}

impl From<WireError> for LockError {
    fn from(wire: WireError) -> Self {
        Self::Wire(wire)
    }
}
