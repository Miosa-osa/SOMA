//! Ending the prepared machines the desired set no longer wants.

use std::collections::HashMap;

use super::{locked, pool, spawn::terminate};

/// Kills every prepared host the desired set no longer wants, and returns how many it ended.
///
/// Machines leave the pool under the same locks a claim takes them under, and are terminated only
/// after those locks are released, so a machine is either claimed whole or retired whole.
pub(super) fn retire_surplus() -> usize {
    let doomed = {
        let desired = locked(&pool().desired);
        let mut available = locked(&pool().available);
        let mut kept_primed: HashMap<String, usize> = HashMap::new();
        let mut kept_generic = 0_usize;
        let mut doomed = Vec::new();
        let mut index = 0;
        while index < available.len() {
            let wanted = match available[index].primed.as_ref() {
                None => {
                    kept_generic += 1;
                    kept_generic <= desired.generic
                }
                Some(key) => desired
                    .targets
                    .iter()
                    .find(|target| &target.key == key)
                    .is_some_and(|target| {
                        let id = format!(
                            "{}\0{}\0{}",
                            key.reference,
                            key.generation_id.as_str(),
                            key.memory_mib
                        );
                        let count = kept_primed.entry(id).or_insert(0);
                        *count += 1;
                        *count <= target.count
                    }),
            };
            if wanted {
                index += 1;
            } else {
                doomed.push(available.swap_remove(index));
            }
        }
        doomed
    };
    let ended = doomed.len();
    for mut host in doomed {
        terminate(&mut host.child);
    }
    ended
}
