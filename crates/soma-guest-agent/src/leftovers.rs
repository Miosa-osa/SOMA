//! Whether the processes a command leaves behind outlive it.
//!
//! By default nothing survives a command: its process group is killed and the sweep ends every
//! other process (see [`crate::descendants`] and [`crate::survivors`]). An Instance whose exec
//! policy names `keep-session` keeps what a command leaves behind when the command ends on its
//! own, as a sandbox does for a server started in the background: the group is not killed, and
//! the sweep spares it until its last member exits. A command that reaches its deadline or its
//! output limit, or whose output could not be delivered, still ends with everything it started.
//!
//! The policy is the Instance's, not the Generation's: the one image serves Instances that keep
//! leftovers and Instances that end them. It is read after each command ends, from a file in
//! `/run`, which every Instance gets fresh.

use std::fs;
use std::sync::{Mutex, PoisonError};

use crate::survivors::Membership;

/// Where an Instance states its exec policy, one word per line.
pub const EXEC_POLICY_FILE: &str = "/run/soma/exec-policy";
const KEEP_SESSION: &str = "keep-session";

/// Process groups kept after their command ended, until none of their members is left.
static KEPT: Mutex<Vec<i32>> = Mutex::new(Vec::new());

/// What becomes of the processes a command leaves behind.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Leftovers {
    /// They end with the command (the default).
    #[default]
    End,
    /// They outlive a command that ended on its own.
    KeepAfterExit,
}

impl Leftovers {
    /// Reads the policy file; a missing or unreadable file is the default.
    #[must_use]
    pub fn read(path: &str) -> Self {
        fs::read_to_string(path).map_or(Self::End, |text| Self::parse(&text))
    }

    /// Parses a policy: `keep-session` on a line of its own keeps leftovers.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        if text.lines().any(|line| line.trim() == KEEP_SESSION) {
            Self::KeepAfterExit
        } else {
            Self::End
        }
    }

    /// Whether a command that ended this way keeps what it left behind.
    #[must_use]
    pub const fn keeps(self, ended_on_its_own: bool) -> bool {
        matches!(self, Self::KeepAfterExit) && ended_on_its_own
    }
}

/// Keeps `group` from the sweep until none of its members is left.
pub fn keep(group: i32) {
    if group <= 1 {
        return;
    }
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    if !kept.contains(&group) {
        kept.push(group);
    }
}

/// Whether any group is kept, so a sweep with nothing listed can skip reading memberships.
#[must_use]
pub fn any_kept() -> bool {
    !KEPT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty()
}

/// The kept groups that still have a member among `processes`; the others are forgotten, so a
/// later process that reuses the identifier is never spared by mistake.
#[must_use]
pub fn live(processes: &[Membership]) -> Vec<i32> {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    retain_live(&mut kept, processes);
    kept.clone()
}

fn retain_live(kept: &mut Vec<i32>, processes: &[Membership]) {
    kept.retain(|group| processes.iter().any(|process| process.group == *group));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_keep_session_line_keeps_leftovers() {
        assert_eq!(Leftovers::parse("keep-session\n"), Leftovers::KeepAfterExit);
        assert_eq!(
            Leftovers::parse("other\n  keep-session  \n"),
            Leftovers::KeepAfterExit
        );
        assert_eq!(Leftovers::parse(""), Leftovers::End);
        assert_eq!(Leftovers::parse("keep-session-later\nkeep"), Leftovers::End);
        assert_eq!(
            Leftovers::read("/nonexistent/soma/exec-policy"),
            Leftovers::End
        );
    }

    #[test]
    fn leftovers_are_kept_only_after_a_command_that_ended_on_its_own() {
        assert!(Leftovers::KeepAfterExit.keeps(true));
        assert!(!Leftovers::KeepAfterExit.keeps(false));
        assert!(!Leftovers::End.keeps(true));
        assert!(!Leftovers::End.keeps(false));
    }

    #[test]
    fn a_kept_group_is_forgotten_once_it_has_no_member() {
        let member = |pid, group| Membership {
            pid,
            group,
            session: 1,
        };
        let mut kept = vec![40, 50];
        retain_live(&mut kept, &[member(41, 40), member(60, 60)]);
        assert_eq!(kept, [40]);
        retain_live(&mut kept, &[member(60, 60)]);
        assert!(kept.is_empty());
    }
}
