//! A prepared root in a scratch directory, and a cache of stand-in values for it.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
    time::{Duration, SystemTime},
};

use soma::GenerationId;

use super::super::super::cache::{Cache, Retained};
use super::super::{Inputs, run};
use crate::{InstanceHolds, PinSource, PrunePlan, PruneReason};

/// A cached value that keeps one descriptor into its entry open, as an admitted Generation does.
#[derive(Clone)]
pub(super) struct Handle(Arc<File>);

impl Retained for Handle {
    fn held_elsewhere(&self) -> bool {
        Arc::strong_count(&self.0) > 1
    }
}

pub(super) struct Host {
    pub(super) scratch: tempfile::TempDir,
    pub(super) cache: Cache<Handle>,
}

impl Host {
    pub(super) fn new() -> Self {
        let scratch = tempfile::tempdir().expect("scratch");
        std::fs::create_dir_all(scratch.path().join("prepared")).expect("root");
        Self {
            scratch,
            cache: Cache::new(),
        }
    }

    pub(super) fn root(&self) -> PathBuf {
        self.scratch.path().join("prepared")
    }

    /// One certified entry for `reference`, published `age_days` ago.
    pub(super) fn certify(&self, reference: &str, fill: u8, age_days: u64) -> PathBuf {
        let entry = self.root().join(format!("ref-{fill:02x}"));
        write_entry(&entry, reference, fill, age_days);
        entry
    }

    /// Caches a value for `reference` holding a descriptor into `entry`.
    pub(super) fn cache(&self, reference: &str, entry: &Path) -> Weak<File> {
        let file = Arc::new(File::open(entry.join("store/root.erofs")).expect("artifact"));
        let weak = Arc::downgrade(&file);
        assert!(self.cache.admit(self.key(reference), Handle(file), entry));
        weak
    }

    pub(super) fn key(&self, reference: &str) -> (PathBuf, String) {
        (self.root(), reference.to_owned())
    }

    pub(super) fn run(
        &self,
        keep: &[&str],
        apply: bool,
        instances: &mut dyn FnMut() -> Result<InstanceHolds, String>,
    ) -> PrunePlan {
        let pins: BTreeMap<String, BTreeSet<PinSource>> = keep
            .iter()
            .map(|reference| ((*reference).to_owned(), BTreeSet::from([PinSource::Keep])))
            .collect();
        let inputs = Inputs {
            root: self.root(),
            head_root: self.scratch.path().join("heads"),
            pins,
            apply,
        };
        run(&inputs, &self.cache, instances).expect("the prune plans")
    }
}

pub(super) fn write_entry(entry: &Path, reference: &str, fill: u8, age_days: u64) {
    std::fs::create_dir_all(entry.join("store")).expect("entry");
    std::fs::write(entry.join("reference"), reference).expect("reference");
    std::fs::write(entry.join("candidate.somacan"), b"candidate").expect("candidate");
    std::fs::write(entry.join("store/root.erofs"), vec![fill; 64 * 1024]).expect("artifact");
    let id = entry.join("generation.id");
    std::fs::write(&id, generation(fill).as_str()).expect("generation.id");
    let published = SystemTime::now() - Duration::from_secs(age_days * 86_400);
    File::options()
        .write(true)
        .open(&id)
        .and_then(|file| file.set_modified(published))
        .expect("publish time");
}

pub(super) fn generation(fill: u8) -> GenerationId {
    GenerationId::new(format!("sha256:{}", format!("{fill:02x}").repeat(32))).expect("generation")
}

pub(super) fn nobody() -> impl FnMut() -> Result<InstanceHolds, String> {
    || Ok(InstanceHolds::default())
}

/// The list an entry landed in, and why.
pub(super) fn verdict(plan: &PrunePlan, name: &str) -> (&'static str, PruneReason) {
    let lists = [
        ("keep", &plan.keep),
        ("remove", &plan.remove),
        ("refused", &plan.refused),
    ];
    lists
        .into_iter()
        .find_map(|(list, entries)| {
            entries
                .iter()
                .find(|entry| entry.entry == name)
                .map(|entry| (list, entry.reason))
        })
        .expect("every entry is in the plan")
}

/// Every path under `root` with its length and modification time.
pub(super) fn tree(root: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for child in std::fs::read_dir(&directory).expect("readable") {
            let path = child.expect("child").path();
            let metadata = std::fs::symlink_metadata(&path).expect("metadata");
            if metadata.is_dir() {
                pending.push(path.clone());
            }
            found.push((path, metadata.len(), metadata.modified().expect("mtime")));
        }
    }
    found.sort();
    found
}
