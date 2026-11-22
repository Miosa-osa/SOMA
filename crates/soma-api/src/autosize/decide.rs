//! The sizing decision: given what a sandbox uses and what it has, what it should have next.
//!
//! Pure, so every rule is a test. It grows at once when the guest is short and shrinks only after
//! the guest has stayed well under its size for a while, so a build that pauses between steps does
//! not lose the room it is about to need again.

use serde::{Deserialize, Serialize};

/// The range one sandbox is kept within.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub min_cpu: u16,
    pub max_cpu: u16,
    pub min_memory_mb: u64,
    pub max_memory_mb: u64,
}

/// One reading of a running sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Sample {
    /// Processors busy over the last interval, in whole-processor units.
    pub cpu_used: f64,
    /// Processors the sandbox may use now.
    pub cpu_limit: u16,
    /// Memory the guest is using: total less what it reports available.
    pub memory_used_mb: u64,
    /// Memory the guest has now.
    pub memory_total_mb: u64,
}

/// What the controller remembers between readings.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Calm {
    /// Readings in a row with the processors mostly idle.
    pub cpu: u32,
    /// Readings in a row with memory well under its size, or with the processors idle and memory
    /// no longer rising.
    pub memory: u32,
    /// Memory used at the first reading after the last memory change. A host reading counts every
    /// page the guest ever touched and does not fall when the guest frees memory, so growth needs
    /// use to have risen since then, not only a high share.
    pub baseline_mb: Option<u64>,
    /// The size the last shrink started from, kept until the next reading shows whether the guest
    /// gave anything back.
    pub shrink_from_mb: Option<u64>,
    /// Readings left before a shrink is asked again, after the guest gave nothing back on the last
    /// one (it held every block). A host reading does not fall when the guest frees memory, so the
    /// wait also ends on its own.
    pub refused: u32,
}

/// Readings in a row (at the controller's two-second interval: thirty seconds) before anything
/// shrinks.
pub const CALM_READINGS: u32 = 15;
/// Readings (two minutes) before a shrink the guest refused is asked again.
pub const REFUSED_READINGS: u32 = 60;
/// Memory is changed in whole blocks of this size.
pub const MEMORY_STEP_MB: u64 = 512;

/// The next size, or `None` for a dimension that stays as it is.
#[must_use]
pub fn next(policy: &Policy, sample: &Sample, calm: &mut Calm) -> (Option<u16>, Option<u64>) {
    (cpu(policy, sample, calm), memory(policy, sample, calm))
}

fn cpu(policy: &Policy, sample: &Sample, calm: &mut Calm) -> Option<u16> {
    let limit = f64::from(sample.cpu_limit.max(1));
    let floor = policy.min_cpu.max(1);
    let ceiling = policy.max_cpu.max(floor);
    // Out of range, as after the policy changed: bring it back first.
    if sample.cpu_limit < floor || sample.cpu_limit > ceiling {
        calm.cpu = 0;
        return Some(sample.cpu_limit.clamp(floor, ceiling));
    }
    if sample.cpu_used >= limit * 0.85 && sample.cpu_limit < ceiling {
        calm.cpu = 0;
        // Doubling reaches a busy build's real need in a few readings instead of one core at a time.
        return Some(sample.cpu_limit.saturating_mul(2).clamp(floor, ceiling));
    }
    if sample.cpu_used < limit * 0.35 && sample.cpu_limit > floor {
        calm.cpu += 1;
        if calm.cpu >= CALM_READINGS {
            calm.cpu = 0;
            let wanted = cores(sample.cpu_used * 2.0).max(sample.cpu_limit / 2);
            return Some(wanted.clamp(floor, ceiling)).filter(|next| *next < sample.cpu_limit);
        }
        return None;
    }
    calm.cpu = 0;
    None
}

fn memory(policy: &Policy, sample: &Sample, calm: &mut Calm) -> Option<u64> {
    let floor = policy.min_memory_mb;
    let ceiling = policy.max_memory_mb.max(floor);
    let total = sample.memory_total_mb.max(1);
    let used = sample.memory_used_mb;
    let baseline = *calm.baseline_mb.get_or_insert(used);
    let rising = used >= baseline + MEMORY_STEP_MB / 2;
    calm.refused = calm.refused.saturating_sub(1);
    if calm.shrink_from_mb.take().is_some_and(|from| total >= from) {
        calm.refused = REFUSED_READINGS;
    }
    if rising || used + MEMORY_STEP_MB / 2 < baseline {
        calm.refused = 0;
    }
    let wanted = round_up(used.saturating_mul(8) / 5).clamp(floor, ceiling);
    // Growing waits for nothing and starts early: memory can fill faster than one reading apart,
    // and a guest that runs out kills what it is building.
    if used * 100 >= total * 65 && total < ceiling && rising {
        calm.memory = 0;
        calm.baseline_mb = None;
        return Some(wanted.max(round_up(total + 1)).min(ceiling));
    }
    let limit = f64::from(sample.cpu_limit.max(1));
    let low = used * 100 < total * 45;
    let idle = sample.cpu_used < limit * 0.35 && !rising;
    if (low || idle) && total > floor && calm.refused == 0 {
        calm.memory += 1;
        if calm.memory >= CALM_READINGS {
            calm.memory = 0;
            // A guest reading says what is used, so the size goes straight to it with room. A host
            // reading cannot tell freed memory from used, so the size steps down a quarter at a
            // time and the guest gives back only blocks nothing holds.
            let step = (total / 4).max(MEMORY_STEP_MB);
            let target = if low {
                wanted
            } else {
                (total.saturating_sub(step) / MEMORY_STEP_MB * MEMORY_STEP_MB).max(floor)
            };
            let next = Some(target).filter(|next| *next + MEMORY_STEP_MB <= total);
            if next.is_some() {
                calm.shrink_from_mb = Some(total);
                calm.baseline_mb = None;
            }
            return next;
        }
        return None;
    }
    calm.memory = 0;
    None
}

fn round_up(mb: u64) -> u64 {
    mb.div_ceil(MEMORY_STEP_MB) * MEMORY_STEP_MB
}

fn cores(used: f64) -> u16 {
    if used.is_finite() && used > 0.0 {
        // Bounded above by the cast: a sandbox has at most u16::MAX processors.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = used.ceil().min(f64::from(u16::MAX)) as u16;
        whole.max(1)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: Policy = Policy {
        min_cpu: 2,
        max_cpu: 16,
        min_memory_mb: 4096,
        max_memory_mb: 32768,
    };

    fn sample(cpu_used: f64, cpu_limit: u16, used: u64, total: u64) -> Sample {
        Sample {
            cpu_used,
            cpu_limit,
            memory_used_mb: used,
            memory_total_mb: total,
        }
    }

    #[test]
    fn a_busy_sandbox_doubles_its_processors_at_once() {
        let mut calm = Calm::default();
        assert_eq!(cpu(&POLICY, &sample(1.9, 2, 0, 4096), &mut calm), Some(4));
        assert_eq!(cpu(&POLICY, &sample(15.0, 16, 0, 4096), &mut calm), None);
        assert_eq!(
            cpu(&POLICY, &sample(11.0, 12, 0, 4096), &mut calm),
            Some(16)
        );
    }

    #[test]
    fn processors_shrink_only_after_a_calm_run() {
        let mut calm = Calm::default();
        for _ in 0..CALM_READINGS - 1 {
            assert_eq!(cpu(&POLICY, &sample(0.2, 8, 0, 4096), &mut calm), None);
        }
        assert_eq!(cpu(&POLICY, &sample(0.2, 8, 0, 4096), &mut calm), Some(4));
        // A busy reading in between starts the count again.
        let mut calm = Calm {
            cpu: 4,
            ..Calm::default()
        };
        assert_eq!(cpu(&POLICY, &sample(4.0, 8, 0, 4096), &mut calm), None);
        assert_eq!(calm.cpu, 0);
    }

    #[test]
    fn memory_grows_when_the_guest_is_short_and_never_past_the_ceiling() {
        let mut calm = Calm {
            baseline_mb: Some(0),
            ..Calm::default()
        };
        assert_eq!(
            memory(&POLICY, &sample(0.0, 2, 3600, 4096), &mut calm),
            Some(6144)
        );
        calm.baseline_mb = Some(0);
        assert_eq!(
            memory(&POLICY, &sample(0.0, 2, 30000, 32768), &mut calm),
            None
        );
        calm.baseline_mb = Some(0);
        assert_eq!(
            memory(&POLICY, &sample(0.0, 2, 27000, 30720), &mut calm),
            Some(32768)
        );
    }

    #[test]
    fn memory_shrinks_after_a_calm_run_to_what_is_used_with_room() {
        let mut calm = Calm::default();
        for _ in 0..CALM_READINGS - 1 {
            assert_eq!(
                memory(&POLICY, &sample(0.0, 2, 2000, 16384), &mut calm),
                None
            );
        }
        assert_eq!(
            memory(&POLICY, &sample(0.0, 2, 2000, 16384), &mut calm),
            Some(4096)
        );
    }

    #[test]
    fn memory_that_stays_high_but_does_not_rise_never_grows() {
        // Host readings keep every page ever touched: a full but steady guest is not short.
        let mut calm = Calm::default();
        for _ in 0..40 {
            assert_ne!(
                memory(&POLICY, &sample(1.5, 2, 4000, 4096), &mut calm).map(|mb| mb > 4096),
                Some(true)
            );
        }
        assert_eq!(
            memory(&POLICY, &sample(1.5, 2, 4200, 4096), &mut calm),
            None,
            "a rise of under half a block is noise"
        );
    }

    #[test]
    fn an_idle_sandbox_steps_its_memory_down_a_quarter_at_a_time() {
        let mut calm = Calm::default();
        let mut asked = None;
        for _ in 0..CALM_READINGS {
            asked = memory(&POLICY, &sample(0.1, 2, 12000, 16384), &mut calm);
        }
        assert_eq!(asked, Some(12288));
        // The guest gave the blocks back: the next run steps again.
        let mut asked = None;
        for _ in 0..CALM_READINGS {
            asked = memory(&POLICY, &sample(0.1, 2, 9000, 12288), &mut calm);
        }
        assert_eq!(asked, Some(9216));
        // Never below the floor.
        let mut asked = None;
        for _ in 0..CALM_READINGS {
            asked = memory(&POLICY, &sample(0.1, 2, 4000, 4608), &mut calm);
        }
        assert_eq!(asked, Some(4096));
    }

    #[test]
    fn a_guest_that_gives_nothing_back_is_asked_again_only_after_a_wait() {
        let mut calm = Calm::default();
        for _ in 0..CALM_READINGS {
            memory(&POLICY, &sample(0.1, 2, 7000, 8192), &mut calm);
        }
        // Still 8192: refused, and not asked again for a while.
        for _ in 0..REFUSED_READINGS {
            assert_eq!(memory(&POLICY, &sample(0.1, 2, 7000, 8192), &mut calm), None);
        }
        // The wait ran out: the next calm run asks again.
        let mut asked = None;
        for _ in 0..CALM_READINGS {
            asked = memory(&POLICY, &sample(0.1, 2, 7000, 8192), &mut calm);
        }
        assert_eq!(asked, Some(6144));
        // A fall in use ends the wait at once.
        memory(&POLICY, &sample(0.1, 2, 7000, 8192), &mut calm);
        assert!(calm.refused > 0);
        memory(&POLICY, &sample(0.1, 2, 6000, 8192), &mut calm);
        assert_eq!(calm.refused, 0);
    }

    #[test]
    fn a_size_outside_the_policy_is_brought_back() {
        let mut calm = Calm::default();
        assert_eq!(cpu(&POLICY, &sample(0.5, 1, 0, 4096), &mut calm), Some(2));
        assert_eq!(cpu(&POLICY, &sample(0.5, 24, 0, 4096), &mut calm), Some(16));
    }
}
