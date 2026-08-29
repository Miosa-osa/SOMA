//! Locates a stable pinned kernel for the live boot test.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const KERNEL_POLL: Duration = Duration::from_secs(60);
const DEFAULT_KERNEL_WAIT: Duration = Duration::from_mins(45);
const STABILITY_WAIT: Duration = Duration::from_secs(10);

/// Output directories a sibling pinned-kernel checkout or an in-repo build may use.
pub fn kernel_candidates() -> Vec<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.join("../..");
    let mut candidates = vec![repo.join("kernel/out")];
    if let Some(parent) = repo
        .canonicalize()
        .ok()
        .and_then(|r| r.parent().map(Path::to_path_buf))
    {
        candidates.push(parent.join("pinned-x86-64-kernel/kernel/out"));
    }
    if let Some(dir) = std::env::var_os("SOMA_X86_64_KERNEL_OUT_DIR") {
        candidates.insert(0, PathBuf::from(dir));
    }
    candidates
}

fn find_kernel_once(candidates: &[PathBuf]) -> Option<PathBuf> {
    for dir in candidates {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("vmlinux-") && name.ends_with("-soma-v1"))
            })
            .collect();
        found.sort();
        if let Some(path) = found.pop() {
            return Some(path);
        }
    }
    None
}

pub fn sha256_of(path: &Path) -> Option<String> {
    let output = Command::new("sha256sum").arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().next().map(str::to_owned)
}

/// True when the file's size is unchanged over `STABILITY_WAIT` and any sibling
/// `manifest.json` names its sha256.
fn kernel_is_stable(path: &Path) -> bool {
    let Ok(first) = fs::metadata(path).map(|m| m.len()) else {
        return false;
    };
    thread::sleep(STABILITY_WAIT);
    let Ok(second) = fs::metadata(path).map(|m| m.len()) else {
        return false;
    };
    if first == 0 || first != second {
        return false;
    }
    let manifest = path.with_file_name("manifest.json");
    match (fs::read_to_string(manifest), sha256_of(path)) {
        (Ok(manifest), Some(sha)) => manifest.contains(&sha),
        (Err(_), Some(_)) => true,
        (_, None) => false,
    }
}

pub fn kernel_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SOMA_X86_64_VMLINUX") {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "SOMA_X86_64_VMLINUX must name an existing kernel"
        );
        return path;
    }
    let wait = std::env::var("SOMA_X86_64_KERNEL_WAIT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .map_or(DEFAULT_KERNEL_WAIT, Duration::from_secs);
    let candidates = kernel_candidates();
    let started = Instant::now();
    loop {
        if let Some(path) = find_kernel_once(&candidates) {
            if kernel_is_stable(&path) {
                return path;
            }
            eprintln!("kernel {} is still changing; waiting", path.display());
        }
        assert!(
            started.elapsed() < wait,
            "prerequisite failed: no stable pinned kernel vmlinux-<ver>-soma-v1 appeared in {candidates:?} within {wait:?}; set SOMA_X86_64_VMLINUX"
        );
        thread::sleep(KERNEL_POLL.min(wait));
    }
}
