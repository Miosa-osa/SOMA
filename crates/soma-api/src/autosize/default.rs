//! Automatic sizing on by default: with [`DEFAULT_ENV`] set, every running sandbox on the host is
//! kept sized without anyone asking, including the ones created before soma-api last started.
//!
//! The controller's own list lives in memory, so a restart (an upgrade, a crash) used to end every
//! sandbox's sizing without a word. Enrolling from the host's own listing closes that: a sandbox
//! the host runs is sized unless it was turned off on purpose with `PUT ... {"enabled": false}`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use soma::{InstanceId, SandboxPhase};

use super::{Policy, sandboxes, set};
use crate::facade::SandboxFacade;

/// Set to `1`, `true` or `on` to size every sandbox on the host by default.
pub const DEFAULT_ENV: &str = "SOMA_AUTOSIZE_DEFAULT";

/// How many controller intervals pass between two readings of the host's listing.
pub const ENROL_EVERY: u32 = 5;

static ON: AtomicBool = AtomicBool::new(false);

static TURNED_OFF: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

fn turned_off() -> std::sync::MutexGuard<'static, HashSet<String>> {
    TURNED_OFF.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `value`, the setting of [`DEFAULT_ENV`], turns default sizing on.
#[must_use]
pub fn setting_is_on(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

/// Turns default sizing on or off for the whole host.
pub fn set_default(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// Whether every sandbox on the host is sized by default.
#[must_use]
pub fn is_default() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Records that `instance` was turned off on purpose, so default sizing leaves it alone.
pub fn turn_off(instance: &InstanceId) {
    turned_off().insert(instance.as_str().to_owned());
}

/// Forgets a deliberate turn-off, because the sandbox was given a range again.
pub fn turn_on(instance: &InstanceId) {
    turned_off().remove(instance.as_str());
}

/// The range a sandbox gets when nobody named one: one processor up to what it booted with, and
/// what it booted with up to its memory ceiling. The same range a `PUT` with no fields asks for.
#[must_use]
pub const fn range_for(cpu_booted: u16, memory_booted_mb: u64, memory_ceiling_mb: u64) -> Policy {
    let max_cpu = if cpu_booted == 0 { 1 } else { cpu_booted };
    Policy {
        min_cpu: 1,
        max_cpu,
        min_memory_mb: memory_booted_mb,
        max_memory_mb: if memory_ceiling_mb > memory_booted_mb {
            memory_ceiling_mb
        } else {
            memory_booted_mb
        },
    }
}

/// Enrols every running sandbox the host lists that is neither sized already nor turned off, and
/// forgets turn-offs for sandboxes that no longer exist. Does nothing unless default sizing is on.
pub fn enrol<F: SandboxFacade + ?Sized>(facade: &mut F) {
    if !is_default() {
        return;
    }
    let Ok(listing) = facade.list() else {
        return;
    };
    let running: Vec<InstanceId> = listing
        .entries()
        .iter()
        .filter(|entry| matches!(entry.phase(), SandboxPhase::Active | SandboxPhase::Executing))
        .map(|entry| entry.instance_id().clone())
        .collect();
    {
        let live: HashSet<&str> = running.iter().map(InstanceId::as_str).collect();
        turned_off().retain(|instance| live.contains(instance.as_str()));
    }
    for instance in running {
        let known = sandboxes().contains_key(instance.as_str());
        if known || turned_off().contains(instance.as_str()) {
            continue;
        }
        if let Ok(current) = facade.resize(&instance, soma_local::ResizeRequest::default()) {
            set(
                &instance,
                Some(range_for(
                    current.cpu.booted,
                    current.memory.booted_mb,
                    current.memory.ceiling_mb,
                )),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_reads_the_usual_words_for_on() {
        for on in ["1", "true", "on", "yes", " ON "] {
            assert!(setting_is_on(Some(on)), "{on}");
        }
        for off in ["0", "false", "off", "", "maybe"] {
            assert!(!setting_is_on(Some(off)), "{off}");
        }
        assert!(!setting_is_on(None));
    }

    #[test]
    fn the_default_range_spans_one_processor_to_the_booted_ones_and_booted_memory_to_the_ceiling() {
        assert_eq!(
            range_for(4, 2048, 16384),
            Policy {
                min_cpu: 1,
                max_cpu: 4,
                min_memory_mb: 2048,
                max_memory_mb: 16384,
            }
        );
    }

    #[test]
    fn a_machine_with_no_memory_headroom_keeps_its_memory_and_never_gets_zero_processors() {
        assert_eq!(
            range_for(0, 4096, 4096),
            Policy {
                min_cpu: 1,
                max_cpu: 1,
                min_memory_mb: 4096,
                max_memory_mb: 4096,
            }
        );
    }
}
