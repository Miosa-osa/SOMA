use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempRoot(PathBuf);

impl TempRoot {
    pub(crate) fn new(label: &str) -> Self {
        let nonce = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "soma-local-test-{}-{label}-{nanos}-{nonce}",
            std::process::id()
        ));
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("soma-local-test-"))
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
