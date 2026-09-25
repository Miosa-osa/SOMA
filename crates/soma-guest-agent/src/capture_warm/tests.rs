use std::fs;
use std::time::{Duration, Instant};

use super::*;

/// Confinement needs `CAP_SYS_ADMIN` to unshare and remount, so these run only as root.
fn root() -> bool {
    // SAFETY: `geteuid` has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

fn shell(script: &str, budget: Duration) -> bool {
    run_one("/bin/sh", &["-c".to_owned(), script.to_owned()], budget)
}

#[test]
fn an_absent_plan_declares_nothing_and_a_bad_one_is_a_fault() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("warm");
    let path = path.to_str().unwrap();
    assert_eq!(read_plan(path), Ok(None));
    fs::write(path, b"/usr/local/bin/node -v\n").unwrap();
    let plan = read_plan(path).unwrap().unwrap();
    assert_eq!(plan.commands()[0].executable(), "/usr/local/bin/node");
    fs::write(path, b"node -v\n").unwrap();
    assert_eq!(read_plan(path), Err(DeclarationFault::Invalid));
    fs::write(path, vec![b'a'; MAX_WARM_PLAN_BYTES + 2]).unwrap();
    assert_eq!(read_plan(path), Err(DeclarationFault::Invalid));
}

#[test]
fn a_warm_command_runs_as_nobody_with_an_empty_environment() {
    if !root() {
        return;
    }
    assert!(shell(
        "test \"$(id -u)\" = 65534 && test \"$(id -g)\" = 65534 && test -z \"$HOME\"",
        COMMAND_BUDGET
    ));
}

#[test]
fn a_warm_command_cannot_write_the_root_proc_or_dev() {
    if !root() {
        return;
    }
    // Each write would succeed for root on a writable mount; confinement must refuse all.
    for script in [
        "touch /capture-warm-probe",
        "echo 1 > /proc/sys/vm/drop_caches",
        "touch /dev/capture-warm-probe",
    ] {
        assert!(!shell(script, COMMAND_BUDGET), "{script} succeeded");
    }
    assert!(!std::path::Path::new("/capture-warm-probe").exists());
    assert!(!std::path::Path::new("/dev/capture-warm-probe").exists());
    // Refusal by account alone would pass the checks above, so the mounts themselves must be
    // read-only in the command's namespace, and only there.
    for mount in ["/", "/dev", "/proc", "/sys"] {
        let script = format!(
            "awk '$5 == \"{mount}\" {{ split($6, o, \",\"); if (o[1] != \"ro\") bad = 1; seen = 1 }} \
             END {{ exit !(seen && !bad) }}' /proc/self/mountinfo"
        );
        assert!(shell(&script, COMMAND_BUDGET), "{mount} was not read-only");
    }
    let mounts = fs::read_to_string("/proc/self/mountinfo").unwrap();
    let own_root = mounts
        .lines()
        .find(|line| line.split(' ').nth(4) == Some("/"))
        .unwrap();
    let before = own_root.split(' ').nth(5).unwrap();
    assert!(
        before.starts_with("rw"),
        "the caller's root options changed: {before}"
    );
}

#[test]
fn an_overrunning_command_is_killed_at_its_budget() {
    if !root() {
        return;
    }
    let started = Instant::now();
    assert!(!run_one(
        "/bin/sleep",
        &["30".to_owned()],
        Duration::from_millis(100)
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_background_process_does_not_outlive_its_command() {
    if !root() {
        return;
    }
    let marker = "31.123";
    assert!(shell(&format!("sleep {marker} & exit 0"), COMMAND_BUDGET));
    // The leader is gone and its group was killed, so no process still carries the marker.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let alive = fs::read_dir("/proc")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| fs::read(entry.path().join("cmdline")).ok())
            .any(|cmdline| {
                cmdline
                    .windows(marker.len())
                    .any(|w| w == marker.as_bytes())
            });
        if !alive {
            break;
        }
        assert!(Instant::now() < deadline, "the background sleep survived");
        std::thread::sleep(Duration::from_millis(20));
    }
}
