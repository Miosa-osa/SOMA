use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    Empty(&'static str),
    ContainsNul(&'static str),
    NotAbsolute(&'static str),
    TooLarge(&'static str),
    Zero(&'static str),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(field) => write!(formatter, "{field} cannot be empty"),
            Self::ContainsNul(field) => write!(formatter, "{field} cannot contain NUL"),
            Self::NotAbsolute(field) => write!(formatter, "{field} must be an absolute path"),
            Self::TooLarge(field) => write!(formatter, "{field} exceeds its maximum"),
            Self::Zero(field) => write!(formatter, "{field} must be non-zero"),
        }
    }
}

impl Error for CommandError {}
