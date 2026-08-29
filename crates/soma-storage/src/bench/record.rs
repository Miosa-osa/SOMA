//! JSONL records written for every sample, summary, template, and run identity.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::cell::Cell;
use super::identity::RunIdentity;
use super::stats::Percentiles;
use crate::fiemap::ExtentSummary;

/// One raw sample of one head creation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// Zero-based burst index inside the cell.
    pub burst: usize,
    /// Zero-based thread index inside the burst.
    pub thread: usize,
    /// True when every phase succeeded.
    pub ok: bool,
    /// Failure description when `ok` is false.
    pub error: Option<String>,
    /// Exclusive creation of the destination, or zero for `cp`.
    pub create_ns: u64,
    /// The `FICLONE` call, or the complete `cp` process for the subprocess method.
    pub clone_ns: u64,
    /// `fsync` of the destination.
    pub file_sync_ns: u64,
    /// `fsync` of the directory.
    pub dir_sync_ns: u64,
    /// Size and extent verification.
    pub verify_ns: u64,
    /// Sum of the phases above.
    pub total_ns: u64,
    /// Wall clock around the complete call including bookkeeping.
    pub wall_ns: u64,
    /// Extents reported by FIEMAP after the clone.
    pub extents: u64,
    /// Extents flagged shared.
    pub shared_extents: u64,
}

impl Sample {
    /// A failed sample with the failure text.
    #[must_use]
    pub fn failed(burst: usize, thread: usize, error: String, wall_ns: u64) -> Self {
        Self {
            burst,
            thread,
            ok: false,
            error: Some(error),
            wall_ns,
            ..Self::default()
        }
    }
}

/// Percentiles of every timed phase of one cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseSummary {
    /// Exclusive creation.
    pub create: Percentiles,
    /// `FICLONE` or the `cp` process.
    pub clone: Percentiles,
    /// File `fsync`.
    pub file_sync: Percentiles,
    /// Directory `fsync`.
    pub dir_sync: Percentiles,
    /// Size and extent verification.
    pub verify: Percentiles,
    /// Concurrent unlink plus directory sync under cleanup pressure; empty otherwise.
    pub unlink: Percentiles,
}

impl PhaseSummary {
    /// Computes every phase over the successful samples and the unlink samples.
    #[must_use]
    pub fn of(samples: &[Sample], unlinks: &[u64]) -> Self {
        let pick = |f: fn(&Sample) -> u64| {
            Percentiles::of(&samples.iter().filter(|s| s.ok).map(f).collect::<Vec<_>>())
        };
        Self {
            create: pick(|s| s.create_ns),
            clone: pick(|s| s.clone_ns),
            file_sync: pick(|s| s.file_sync_ns),
            dir_sync: pick(|s| s.dir_sync_ns),
            verify: pick(|s| s.verify_ns),
            unlink: Percentiles::of(unlinks),
        }
    }
}

/// Identity of one template used by the matrix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRecord {
    /// Size label.
    pub size: super::cell::TemplateSize,
    /// Allocation label.
    pub allocation: super::cell::Allocation,
    /// Overlay class name used for the template.
    pub class: String,
    /// SHA-256 of the template bytes.
    pub digest: String,
    /// Logical bytes.
    pub bytes: u64,
    /// Extent summary of the template.
    pub extents: ExtentSummary,
    /// Wall time of template creation in nanoseconds, outside every timed sample.
    pub creation_ns: u64,
}

/// One JSONL line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "kebab-case")]
pub enum Record {
    /// First line: the run identity.
    Identity(RunIdentity),
    /// One template.
    Template(TemplateRecord),
    /// One raw sample.
    Sample {
        /// Cell identifier.
        cell: String,
        /// Cell dimensions.
        #[serde(flatten)]
        dimensions: Cell,
        /// The sample.
        #[serde(flatten)]
        sample: Sample,
    },
    /// One unlink sample recorded under cleanup pressure.
    Unlink {
        /// Cell identifier.
        cell: String,
        /// Burst index.
        burst: usize,
        /// Thread index.
        thread: usize,
        /// True when the unlink and directory sync succeeded.
        ok: bool,
        /// Wall clock of unlink plus directory sync.
        total_ns: u64,
    },
    /// Per-cell summary written after its last burst.
    Summary {
        /// Cell identifier.
        cell: String,
        /// Cell dimensions.
        #[serde(flatten)]
        dimensions: Cell,
        /// Successful samples.
        ok: usize,
        /// Failed samples.
        failed: usize,
        /// Percentiles of `total_ns`.
        total: Percentiles,
        /// Percentiles of `clone_ns`.
        clone: Percentiles,
        /// Percentiles of every phase.
        phases: PhaseSummary,
    },
}

/// Buffered JSONL writer that creates its file exclusively.
pub struct Writer {
    inner: BufWriter<File>,
}

impl Writer {
    /// Creates `path` exclusively.
    ///
    /// # Errors
    ///
    /// Propagates the creation failure, including an existing file.
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Self {
            inner: BufWriter::new(file),
        })
    }

    /// Appends one record as one line.
    ///
    /// # Errors
    ///
    /// Propagates serialization and write failures.
    pub fn write(&mut self, record: &Record) -> io::Result<()> {
        serde_json::to_writer(&mut self.inner, record)?;
        self.inner.write_all(b"\n")
    }

    /// Flushes and syncs the file.
    ///
    /// # Errors
    ///
    /// Propagates the flush or sync failure.
    pub fn finish(mut self) -> io::Result<()> {
        self.inner.flush()?;
        self.inner.get_ref().sync_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::cell::{Allocation, CacheState, Method, Pressure, TemplateSize};

    #[test]
    fn sample_records_flatten_cell_and_sample_fields() {
        let cell = Cell {
            template_size: TemplateSize::Gib1,
            allocation: Allocation::Preallocated,
            cache: CacheState::Cold,
            concurrency: 10,
            pressure: Pressure::None,
            method: Method::Ficlone,
        };
        let record = Record::Sample {
            cell: cell.id(),
            dimensions: cell,
            sample: Sample {
                ok: true,
                total_ns: 42,
                ..Sample::default()
            },
        };
        let json = serde_json::to_string(&record).expect("json");
        assert!(json.contains("\"record\":\"sample\""));
        assert!(json.contains("\"template_size\":\"gib1\""));
        assert!(json.contains("\"allocation\":\"preallocated\""));
        assert!(json.contains("\"total_ns\":42"));
        let parsed: Record = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed, record);
    }

    #[test]
    fn writer_refuses_to_overwrite_and_writes_lines() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("out.jsonl");
        let mut writer = Writer::create(&path).expect("create");
        writer
            .write(&Record::Unlink {
                cell: "x".into(),
                burst: 0,
                thread: 1,
                ok: true,
                total_ns: 5,
            })
            .expect("write");
        writer.finish().expect("finish");
        assert!(Writer::create(&path).is_err());
        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with("{\"record\":\"unlink\""));
    }
}
