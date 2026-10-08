//! Writing one sandbox's internal timeline out, when an operator asks for it.
//!
//! The receipt carries the milestones a caller is owed: accepted, machine launched, ready,
//! command finished. Those are the right public evidence, and they are too coarse to optimise
//! against, because everything between mapping guest memory and the guest agent answering its
//! first probe arrives as one number. The machine already records a much finer timeline for its
//! own evidence, and cleanup already receives it, so nothing needs to be measured again: it only
//! needs somewhere to go.
//!
//! `SOMA_KVM_TIMELINE` names a directory. When it is set, each sandbox writes one JSON file there
//! at cleanup holding its milestone offsets and per-phase durations. When it is unset nothing is
//! written and nothing is formatted, so this costs an environment lookup per sandbox and is safe
//! to leave compiled in.
//! A failed sandbox also writes at most the final 64 KiB of guest console output, which may contain
//! workload data, so only a trusted operator may enable the directory and collect its contents.
//!
//! This is a diagnostic, not evidence. The file has no signature, no identity binding, and no
//! stable schema, and it must never be quoted as a measurement of record: the retained benchmark
//! artifacts remain the only thing that supports a claim.

use std::fmt::Write as _;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use soma_kvm::x86_64::{Milestone, MilestoneMark, SandboxEvidence};

/// The directory each sandbox's timeline is written to, when an operator names one.
const TIMELINE_DIR: &str = "SOMA_KVM_TIMELINE";
const MAX_CONSOLE_BYTES: usize = 64 * 1024;

/// Writes the timeline of a sandbox that failed before it could be shut down.
///
/// A failure is the case the timeline is most needed for and the only one that never reached
/// cleanup, so the guest console is written beside it: which line the guest last printed is
/// usually the whole diagnosis, and it is lost otherwise.
pub(super) fn dump_failure(instance: &str, evidence: &SandboxEvidence, error: &str) {
    let Some(directory) = directory() else {
        return;
    };
    let Some(instance) = safe_instance(instance) else {
        return;
    };
    let _ignored = write_new(
        &directory.join(format!("{instance}.failed.json")),
        render_failure(evidence, error).as_bytes(),
    );
    let start = evidence.serial.len().saturating_sub(MAX_CONSOLE_BYTES);
    let _ignored = write_new(
        &directory.join(format!("{instance}.console")),
        &evidence.serial[start..],
    );
}

/// The directory timelines are written to, created on demand, when an operator named one.
fn directory() -> Option<PathBuf> {
    let directory = std::env::var_os(TIMELINE_DIR).map(PathBuf::from)?;
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory)
}

/// Writes `evidence`'s timeline for `instance`, if a directory was named.
///
/// Every failure is ignored: a diagnostic that cannot be written must never turn a sandbox that
/// ran correctly into one that reports a cleanup failure.
pub fn dump(instance: &str, evidence: &SandboxEvidence) {
    let Some(directory) = directory() else {
        return;
    };
    let Some(instance) = safe_instance(instance) else {
        return;
    };
    let _ignored = write_new(
        &directory.join(format!("{instance}.json")),
        render(evidence).as_bytes(),
    );
}

/// Accepts only a bounded hexadecimal Instance identity as a diagnostic filename.
fn safe_instance(instance: &str) -> Option<&str> {
    (instance.len() == 32 && instance.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(instance)
}

/// Creates one private diagnostic without following or overwriting an existing final path.
fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

/// Renders the timeline as one JSON object of milestone and phase names to nanoseconds.
///
/// Names come from the enums' own `Debug`, which is what those variants are called in the source.
/// A duplicate milestone keeps its first offset, because a milestone that is marked twice is
/// interesting as the moment it was first reached.
fn render(evidence: &SandboxEvidence) -> String {
    let mut out = String::from("{\"milestones_ns\":{");
    let mut first = true;
    let mut seen = Vec::new();
    for mark in &evidence.timeline {
        let name = format!("{:?}", mark.milestone);
        if seen.contains(&name) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        let _ignored = write!(out, "\"{name}\":{}", mark.elapsed_ns);
        seen.push(name);
    }
    out.push('}');
    let split = ready_split(&evidence.timeline);
    if !split.is_empty() {
        out.push_str(",\"ready_split_ns\":{");
        for (index, (name, elapsed)) in split.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ignored = write!(out, "\"{name}\":{elapsed}");
        }
        out.push('}');
    }
    out.push_str(",\"phases_ns\":{");
    for (index, timing) in evidence.phases.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ignored = write!(out, "\"{:?}\":{}", timing.phase(), timing.elapsed_ns());
    }
    out.push_str("}}\n");
    out
}

/// The ordered steps between resuming the guest and `Ready`, as named spans in nanoseconds.
///
/// Each span runs from the previous step that was recorded to the next, so a step a particular
/// path never marks folds into its neighbour instead of failing the dump. `page_consumed` is
/// observed by a host poll and is therefore late by up to one poll interval plus timer slack;
/// it is an upper bound on when the guest took the page, and the later spans absorb the same
/// amount in reverse.
fn ready_split(timeline: &[MilestoneMark]) -> Vec<(&'static str, u64)> {
    const STEPS: [(&str, Milestone); 6] = [
        ("page_consumed", Milestone::LaunchPageConsumed),
        ("vsock_connected", Milestone::VsockConnected),
        ("handshake_done", Milestone::Handshake),
        ("repair_reported", Milestone::RepairReported),
        ("page_retired", Milestone::LaunchPageRetired),
        ("ready", Milestone::Ready),
    ];
    let at = |wanted: Milestone| {
        timeline
            .iter()
            .find(|mark| mark.milestone == wanted)
            .map(|mark| mark.elapsed_ns)
    };
    let (Some(start), Some(end)) = (at(Milestone::RunStart), at(Milestone::Ready)) else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    let mut previous = start;
    for (name, milestone) in STEPS {
        if let Some(now) = at(milestone) {
            spans.push((name, now.saturating_sub(previous)));
            previous = now;
        }
    }
    spans.push(("run_start_to_ready", end.saturating_sub(start)));
    spans
}

/// Renders a failed sandbox's timeline, naming the step that failed.
fn render_failure(evidence: &SandboxEvidence, error: &str) -> String {
    let body = render(evidence);
    let head = body.trim_end().trim_end_matches('}');
    append_error(head, error)
}

fn append_error(head: &str, error: &str) -> String {
    format!("{head},\"error\":\"{}\"}}\n", escaped(error))
}

/// Escapes one string for a JSON document.
///
/// The error text comes from a failed sandbox, so it may hold quotes, backslashes, and control
/// characters. A diagnostic that stopped being parseable at the moment it mattered most would
/// be worse than none, so every byte JSON reserves is escaped rather than trusted.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if control < ' ' || control == '\u{7f}' => {
                use std::fmt::Write as _;
                let _ignored = write!(out, "\\u{:04x}", u32::from(control));
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use soma_kvm::x86_64::{Milestone, MilestoneMark};

    use super::{append_error, ready_split, safe_instance};

    fn mark(milestone: Milestone, elapsed_ns: u64) -> MilestoneMark {
        MilestoneMark {
            milestone,
            elapsed_ns,
        }
    }

    #[test]
    fn the_ready_segment_is_split_at_every_recorded_step() {
        let timeline = [
            mark(Milestone::RunStart, 1_000),
            mark(Milestone::LaunchPageConsumed, 1_900),
            mark(Milestone::VsockConnected, 2_500),
            mark(Milestone::Handshake, 3_100),
            mark(Milestone::RepairReported, 4_000),
            mark(Milestone::LaunchPageRetired, 4_700),
            mark(Milestone::Ready, 4_900),
        ];
        assert_eq!(
            ready_split(&timeline),
            vec![
                ("page_consumed", 900),
                ("vsock_connected", 600),
                ("handshake_done", 600),
                ("repair_reported", 900),
                ("page_retired", 700),
                ("ready", 200),
                ("run_start_to_ready", 3_900),
            ]
        );
    }

    #[test]
    fn a_step_a_path_never_marks_folds_into_the_next_span() {
        let timeline = [
            mark(Milestone::RunStart, 0),
            mark(Milestone::Handshake, 500),
            mark(Milestone::Ready, 800),
        ];
        let split = ready_split(&timeline);
        assert_eq!(split[0], ("handshake_done", 500));
        assert_eq!(split[1], ("ready", 300));
    }

    #[test]
    fn no_split_without_both_ends() {
        assert!(ready_split(&[mark(Milestone::RunStart, 0)]).is_empty());
        assert!(ready_split(&[mark(Milestone::Ready, 9)]).is_empty());
    }

    #[test]
    fn hostile_error_text_remains_valid_json() {
        let rendered = append_error("{\"milestones_ns\":{},\"phases_ns\":{}", "bad \"x\"\\\n");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!(value["error"], "bad \"x\"\\\n");
    }

    #[test]
    fn only_canonical_instance_names_reach_the_filesystem() {
        assert!(safe_instance("89db112753324c3e890ef78b74381aa5").is_some());
        assert!(safe_instance("../tenant-secret").is_none());
        assert!(safe_instance("89DB112753324C3E890EF78B74381AA5").is_some());
    }
}
