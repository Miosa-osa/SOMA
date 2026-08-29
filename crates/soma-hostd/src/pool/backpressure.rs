//! Pool limits and the explicit rejections a saturated pool returns instead of queueing.

use std::{fmt, time::Duration};

use crate::PoolKeyDigest;

/// What a claim does when no sterile worker exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExhaustedBehavior {
    /// Return [`Exhausted`] immediately; the caller may use the separately measured
    /// on-demand path.
    Reject,
    /// Construct one worker inline within the construction deadline and label the claim
    /// as on-demand; there is still no queue.
    ConstructInline,
}

/// The bounded policy of one pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Sterile workers below which replenishment is urgent.
    pub min: usize,
    /// Sterile plus constructing workers replenishment aims for.
    pub target: usize,
    /// Live workers in every nonterminal phase the pool may hold.
    pub max: usize,
    /// Concurrent constructions.
    pub replenish_concurrency: usize,
    /// How long a claim may wait for an in-flight replay and how long a claimed worker may
    /// stay unassigned before it is destroyed.
    pub claim_deadline: Duration,
    /// How long one construction may take.
    pub construction_deadline: Duration,
    /// What happens when no sterile worker exists.
    pub exhausted: ExhaustedBehavior,
    /// Retained claim bindings for idempotent replay.
    pub binding_limit: usize,
}

/// Why a limits value is not usable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitsError {
    /// `min <= target <= max` was violated.
    Ordering,
    /// `max` was zero.
    ZeroMaximum,
    /// The replenishment concurrency was zero.
    ZeroConcurrency,
    /// A deadline was zero.
    ZeroDeadline,
    /// The binding limit was below `max`.
    BindingLimit,
}

impl fmt::Display for LimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ordering => formatter.write_str("limits require min <= target <= max"),
            Self::ZeroMaximum => formatter.write_str("pool maximum is zero"),
            Self::ZeroConcurrency => formatter.write_str("replenish concurrency is zero"),
            Self::ZeroDeadline => formatter.write_str("a deadline is zero"),
            Self::BindingLimit => formatter.write_str("binding limit is below the pool maximum"),
        }
    }
}

impl std::error::Error for LimitsError {}

impl Limits {
    /// Validates the policy.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    pub const fn validate(self) -> Result<Self, LimitsError> {
        if self.max == 0 {
            return Err(LimitsError::ZeroMaximum);
        }
        if self.min > self.target || self.target > self.max {
            return Err(LimitsError::Ordering);
        }
        if self.replenish_concurrency == 0 {
            return Err(LimitsError::ZeroConcurrency);
        }
        if self.claim_deadline.is_zero() || self.construction_deadline.is_zero() {
            return Err(LimitsError::ZeroDeadline);
        }
        if self.binding_limit < self.max {
            return Err(LimitsError::BindingLimit);
        }
        Ok(self)
    }
}

/// Workers per phase at one instant.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Occupancy {
    /// Workers being built.
    pub constructing: usize,
    /// Claimable workers.
    pub sterile: usize,
    /// Workers in transfer.
    pub claiming: usize,
    /// Assigned workers.
    pub assigned: usize,
    /// Running workers.
    pub running: usize,
    /// Workers being torn down.
    pub destroying: usize,
    /// Closed workers still in the table.
    pub dead: usize,
}

impl Occupancy {
    /// Workers in every nonterminal phase.
    #[must_use]
    pub const fn live(&self) -> usize {
        self.constructing
            + self.sterile
            + self.claiming
            + self.assigned
            + self.running
            + self.destroying
    }

    /// Workers that count toward the replenishment target.
    #[must_use]
    pub const fn prepared(&self) -> usize {
        self.constructing + self.sterile
    }
}

/// No sterile worker was available; nothing was queued.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Exhausted {
    /// The pool.
    pub key: PoolKeyDigest,
    /// What the pool held at rejection time.
    pub occupancy: Occupancy,
    /// The configured maximum.
    pub max: usize,
    /// The configured behavior that produced this rejection.
    pub behavior: ExhaustedBehavior,
}

/// Which bounded structure refused more work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverloadGate {
    /// The idempotent claim registry has no room for another live binding.
    ClaimRegistry,
    /// The slot table already holds a live worker with this identity.
    DuplicateWorker,
    /// Every replenishment slot is busy.
    ReplenishConcurrency,
    /// The pool holds `max` live workers.
    PoolMaximum,
}

/// A bounded structure refused more work.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Overloaded {
    /// The full structure.
    pub gate: OverloadGate,
    /// Its current fill.
    pub current: usize,
    /// Its limit.
    pub limit: usize,
}

impl fmt::Display for Exhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pool {:?} exhausted: {} sterile, {} constructing, {} live of {} ({:?})",
            self.key,
            self.occupancy.sterile,
            self.occupancy.constructing,
            self.occupancy.live(),
            self.max,
            self.behavior
        )
    }
}

impl fmt::Display for Overloaded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} overloaded: {} of {}",
            self.gate, self.current, self.limit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn limits() -> Limits {
        Limits {
            min: 1,
            target: 2,
            max: 4,
            replenish_concurrency: 2,
            claim_deadline: Duration::from_millis(50),
            construction_deadline: Duration::from_millis(50),
            exhausted: ExhaustedBehavior::Reject,
            binding_limit: 8,
        }
    }

    #[test]
    fn limits_validate_every_rule() {
        assert!(limits().validate().is_ok());
        assert_eq!(
            Limits { max: 0, ..limits() }.validate(),
            Err(LimitsError::ZeroMaximum)
        );
        assert_eq!(
            Limits {
                target: 5,
                ..limits()
            }
            .validate(),
            Err(LimitsError::Ordering)
        );
        assert_eq!(
            Limits {
                replenish_concurrency: 0,
                ..limits()
            }
            .validate(),
            Err(LimitsError::ZeroConcurrency)
        );
        assert_eq!(
            Limits {
                claim_deadline: Duration::ZERO,
                ..limits()
            }
            .validate(),
            Err(LimitsError::ZeroDeadline)
        );
        assert_eq!(
            Limits {
                binding_limit: 3,
                ..limits()
            }
            .validate(),
            Err(LimitsError::BindingLimit)
        );
        let occupancy = Occupancy {
            constructing: 1,
            sterile: 2,
            dead: 9,
            ..Occupancy::default()
        };
        assert_eq!(occupancy.live(), 3);
        assert_eq!(occupancy.prepared(), 3);
    }
}
