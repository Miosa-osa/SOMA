//! Paging the workload's runtime into guest memory before the capture point.
//!
//! A restored sandbox starts from captured guest memory, so whatever was resident when the
//! snapshot was taken is resident again immediately, for every Instance, at no cost. Whatever was
//! not resident must be faulted in from the immutable root by each Instance separately.
//!
//! Measured on a `node:22` Generation: the first `node -v` inside a fresh sandbox took 70 ms and
//! the second took 5 ms. That 65 ms difference is page-in work, not computation, and it was being
//! paid once per sandbox because the capture happens before the runtime is ever touched.
//!
//! So the agent reads a warm list before it announces the repair point. Each listed file is read
//! once, which populates the guest page cache, and the capture then records those pages. Nothing
//! is executed: reading a file cannot run it, so this adds no code path to the boot and no
//! identity to the captured memory.
//!
//! Which files to warm should come from the Generation, because the compiler knows the image's
//! declared entrypoint. It does not yet, and the overlay cannot carry the list: the agent
//! requires a sterile upper layer and refuses to boot if anything has been placed there, which is
//! a check worth keeping. So this version warms a conventional set of runtime paths, warming only
//! those that exist. That is a heuristic, and it is the part to replace with a compiler-emitted
//! list rather than to extend with more names.
//!
//! Warming is advisory throughout. A missing path or an unreadable one is not an error, because a
//! Generation that cannot be warmed must still boot.

use std::fs;
use std::io::Read as _;

/// Where the composed root may carry an explicit list of paths to warm, one per line.
///
/// Nothing emits this yet. It is read first so that a Generation which does declare its runtime
/// overrides the conventional list below without another agent change.
const WARM_LIST: &str = "/etc/soma/warm";

/// Runtime entrypoints warmed when the Generation declares no list of its own.
///
/// These are the interpreter binaries an OCI image of that runtime places at a fixed path. Only
/// those that exist are read, so an image carrying none of them simply warms nothing.
const CONVENTIONAL: &[&str] = &[
    "/usr/local/bin/node",
    "/usr/bin/node",
    "/usr/local/bin/python3",
    "/usr/bin/python3",
    "/bin/busybox",
];
/// Most bytes read from any one listed file.
///
/// A runtime's own text is the part that matters, and this bounds a hostile or mistaken entry
/// naming something enormous from spending the whole boot budget. It is sized to hold the largest
/// single file a default image warms: the Chromium binary, which is about 290 MiB.
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Most entries honoured, so a long list cannot stall the boot either.
///
/// A browser links about a hundred shared libraries, and the list names the resident ones.
const MAX_ENTRIES: usize = 256;

/// Reads every file the warm list names, returning how many were read.
///
/// Reading is the whole point: the bytes are discarded, and what remains is the guest page cache
/// the snapshot will capture.
pub fn runtime() -> usize {
    match fs::read_to_string(WARM_LIST) {
        Ok(list) => warm_listed(&list),
        // No declared list, so fall back to the conventional runtime paths.
        Err(_) => CONVENTIONAL
            .iter()
            .filter(|path| read_through(path))
            .count(),
    }
}

/// Warms every path an explicit list names.
fn warm_listed(list: &str) -> usize {
    let mut warmed = 0;
    for line in list.lines().take(MAX_ENTRIES) {
        let path = line.trim();
        // Absolute paths only: the agent's working directory at this point is not a meaningful
        // base, so a relative entry is a mistake rather than a shorthand.
        if path.is_empty() || path.starts_with('#') || !path.starts_with('/') {
            continue;
        }
        if read_through(path) {
            warmed += 1;
        }
    }
    warmed
}

/// Reads one file to the bound, discarding the bytes. Returns whether anything was read.
fn read_through(path: &str) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    // Heap rather than stack: PID 1 runs on a modest stack and a large local buffer here would
    // be the one allocation that overruns it.
    let mut sink = vec![0_u8; 128 * 1024];
    let mut bounded = file.take(MAX_FILE_BYTES);
    let mut any = false;
    loop {
        match bounded.read(&mut sink) {
            Ok(0) | Err(_) => return any,
            Ok(_) => any = true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_warms_only_absolute_paths_that_exist_and_skips_comments() {
        let dir = std::env::temp_dir().join(format!("soma-warm-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch directory");
        let real = dir.join("real.bin");
        fs::write(&real, vec![7_u8; 300_000]).expect("fixture");
        let list = format!(
            "# a comment\n\n{}\nrelative/path\n/soma/no/such/file\n  {}  \n",
            real.display(),
            real.display()
        );
        assert_eq!(warm_listed(&list), 2);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_list_is_cut_off_at_the_entry_bound() {
        let dir = std::env::temp_dir().join(format!("soma-warm-cap-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch directory");
        let real = dir.join("real.bin");
        fs::write(&real, b"x").expect("fixture");
        let list = format!("{}\n", real.display()).repeat(MAX_ENTRIES + 40);
        assert_eq!(warm_listed(&list), MAX_ENTRIES);
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_file_larger_than_the_sink_is_read_through_in_full() {
        let dir = std::env::temp_dir().join(format!("soma-warm-big-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch directory");
        let big = dir.join("big.bin");
        fs::write(&big, vec![1_u8; 1_000_000]).expect("fixture");
        assert!(read_through(big.to_str().expect("utf8 path")));
        assert!(!read_through("/soma/no/such/file"));
        fs::remove_dir_all(&dir).expect("cleanup");
    }
}
