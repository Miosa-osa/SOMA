//! The vocabulary one reconciliation pass answers in: what it failed to release, why, and
//! whether that leaves anything behind.
//!
//! A caller cannot retry usefully without knowing which resource survived and with which errno,
//! so a pass never reports a bare success or failure. This is the part other crates match on and
//! print, which is why it is kept apart from the releasing itself.

use core::fmt;

/// A resource that survived reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResidualKind {
    Process,
    Cgroup,
    JailRoot,
}

/// One residual with the errno that prevented its release.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Residual {
    pub kind: ResidualKind,
    pub errno: i32,
}

/// The outcome of one reconciliation pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Disposition {
    Released,
    Incomplete { residuals: Vec<Residual> },
}

impl Disposition {
    #[must_use]
    pub fn is_released(&self) -> bool {
        matches!(self, Self::Released)
    }

    pub(super) fn from_residuals(residuals: Vec<Residual>) -> Self {
        if residuals.is_empty() {
            Self::Released
        } else {
            Self::Incomplete { residuals }
        }
    }
}

impl fmt::Display for Disposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Released => write!(formatter, "released"),
            Self::Incomplete { residuals } => {
                write!(formatter, "incomplete:")?;
                for residual in residuals {
                    write!(formatter, " {:?}(errno {})", residual.kind, residual.errno)?;
                }
                Ok(())
            }
        }
    }
}
