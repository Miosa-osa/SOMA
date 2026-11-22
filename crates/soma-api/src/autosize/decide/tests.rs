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
