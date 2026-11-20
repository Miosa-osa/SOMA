//! T2 seen from the workload's side: a command never runs in the agent's realtime band.
//!
//! The command reports its own scheduling policy, read by the kernel for the process that
//! actually runs, so the case holds whatever mechanism keeps the band away from the child.

use std::fs;

use soma_guest::TerminalStatus;

use super::{command, run};
use crate::priority::{self, Priority};

/// `rt_priority` and `policy` are fields 40 and 41 of `/proc/<pid>/stat`.
const RT_PRIORITY_FIELD: usize = 40;
const POLICY_FIELD: usize = 41;

/// The `(policy, rt_priority)` pair one `stat` line reports.
fn scheduling(stat: &[u8]) -> (u32, u32) {
    let text = std::str::from_utf8(stat).expect("a stat line is text");
    // Field 2 is the command name in parentheses and may itself hold spaces or parentheses, so
    // the fields are counted from the last closing parenthesis, where field 3 begins.
    let rest = &text[text.rfind(')').expect("a stat line names its command") + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let field = |number: usize| -> u32 {
        fields[number - 3]
            .parse()
            .expect("a numeric scheduling field")
    };
    (field(POLICY_FIELD), field(RT_PRIORITY_FIELD))
}

/// Puts the test thread back at the default policy, as it was before the case raised it.
struct Lowered;

impl Drop for Lowered {
    fn drop(&mut self) {
        let _ = priority::drop_to_default();
    }
}

#[test]
fn a_command_runs_at_the_default_policy_while_the_agent_holds_the_realtime_band() {
    let raised = priority::raise();
    let _lowered = Lowered;
    if !matches!(raised, Priority::Realtime(_)) {
        // Without CAP_SYS_NICE the band cannot be taken, so there is nothing a child could
        // inherit and this host cannot show the guarantee either way.
        eprintln!("skipped: the kernel refused the realtime band here ({raised})");
        return;
    }
    let own = fs::read("/proc/thread-self/stat").expect("the test thread's own stat");
    let fifo = u32::try_from(libc::SCHED_FIFO).expect("a small policy number");
    let priority = u32::try_from(priority::PRIORITY).expect("a small priority");
    assert_eq!(
        scheduling(&own),
        (fifo, priority),
        "the agent side runs in the realtime band"
    );

    let (completion, sink) = run(&command("/bin/cat", &["/proc/self/stat"], 5_000, 4096));

    assert_eq!(completion.status, TerminalStatus::Exited(0));
    let other = u32::try_from(libc::SCHED_OTHER).expect("a small policy number");
    assert_eq!(
        scheduling(&sink.stdout),
        (other, 0),
        "a command must start at the default policy, never in the agent's realtime band"
    );
}
