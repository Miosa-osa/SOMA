//! The ledger directory's group commit.
//!
//! Unix only: a directory cannot be opened as a file to sync it on Windows.

use std::{fs::File, io, path::Path};

/// Group commit for the ledger directory.
///
/// Every record is hard-linked into place, and the link is durable only after the directory is
/// synced. Concurrent claims each used to pay their own directory sync; they now share one: a
/// writer takes a ticket after its link exists, and one writer syncs on behalf of every ticket
/// issued before the sync began. A writer returns only when a sync that started after its link
/// has finished, so the durability each record had before is unchanged.
#[derive(Debug, Default)]
pub(super) struct DirectorySync {
    state: std::sync::Mutex<SyncState>,
    done: std::sync::Condvar,
}

#[derive(Debug, Default)]
struct SyncState {
    issued: u64,
    synced: u64,
    running: bool,
}

impl DirectorySync {
    pub(super) fn commit(&self, root: &Path) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.issued += 1;
        let ticket = state.issued;
        loop {
            if state.synced >= ticket {
                return Ok(());
            }
            if state.running {
                state = self
                    .done
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                continue;
            }
            state.running = true;
            let covers = state.issued;
            drop(state);
            let outcome = File::open(root).and_then(|directory| directory.sync_all());
            state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.running = false;
            if outcome.is_ok() {
                state.synced = covers;
            }
            self.done.notify_all();
            outcome?;
        }
    }
}
