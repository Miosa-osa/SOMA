use std::{error::Error, fmt};

macro_rules! validated_id {
    ($name:ident, $length:literal, $label:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name([u8; $length]);

        impl $name {
            /// Creates a validated identifier from its fixed-width representation.
            ///
            /// # Errors
            ///
            /// Returns [`IdError::AllZero`] when every byte is zero.
            pub fn new(bytes: [u8; $length]) -> Result<Self, IdError> {
                if bytes.iter().all(|byte| *byte == 0) {
                    Err(IdError::AllZero($label))
                } else {
                    Ok(Self(bytes))
                }
            }

            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; $length] {
                &self.0
            }
        }
    };
}

validated_id!(OperationId, 16, "operation");
validated_id!(InstanceId, 16, "instance");
validated_id!(GenerationId, 32, "generation");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdError {
    AllZero(&'static str),
}

impl fmt::Display for IdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AllZero(kind) => write!(formatter, "{kind} identifier cannot be all zero"),
        }
    }
}

impl Error for IdError {}
