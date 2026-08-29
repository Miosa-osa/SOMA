//! Retained benchmark matrix for the on-demand versus prepared-head decision.
//!
//! The matrix crosses template size, allocated extent count, cache state, concurrency,
//! free-space pressure, and cleanup pressure, and compares in-process `FICLONE` with the
//! `cp --reflink=always` subprocess that the current Miosa host uses.
//! Every raw duration is written as one JSONL record together with the kernel, filesystem,
//! mount, device, CPU, and template identity so the evidence document can be regenerated.

pub mod burst;
pub mod cell;
pub mod identity;
pub mod matrix;
pub mod pressure;
pub mod record;
pub mod report;
pub mod stats;
pub mod templates;

use std::fmt;
use std::io;
use std::path::PathBuf;

use cell::{Cell, Method};
use stats::Percentiles;

/// Disk-head share of the fresh-resource-activation budget from `docs/architecture/fast-path.md`,
/// `below 1.00 ms` at p99.
pub const DEFAULT_BUDGET_NS: u64 = 1_000_000;

/// Complete configuration of one benchmark run.
#[derive(Clone, Debug)]
pub struct BenchConfig {
    /// Root of the XFS reflink mount; `templates/` and `heads/` are created inside it.
    pub dir: PathBuf,
    /// Destination JSONL file, created exclusively.
    pub out: PathBuf,
    /// Samples per cell.
    pub samples: usize,
    /// Restrict the matrix to the smoke subset.
    pub quick: bool,
    /// p99 budget in nanoseconds for the decision.
    pub budget_ns: u64,
}

/// Summary of one cell after every burst completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSummary {
    /// The cell.
    pub cell: Cell,
    /// Successful samples.
    pub ok: usize,
    /// Failed samples.
    pub failed: usize,
    /// Percentiles of the complete clone cost.
    pub total: Percentiles,
    /// Percentiles of the `FICLONE` call or the `cp` process alone.
    pub clone: Percentiles,
    /// Percentiles of every phase.
    pub phases: record::PhaseSummary,
}

/// The decision the matrix supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Budget applied.
    pub budget_ns: u64,
    /// Worst p99 of the complete clone cost over every 100-way `FICLONE` cell.
    pub worst_p99_ns: u64,
    /// Cell that produced the worst p99.
    pub worst_cell: Option<Cell>,
    /// Samples that failed anywhere in the matrix.
    pub failures: usize,
    /// True only when every 100-way `FICLONE` cell fits the budget with no failure.
    pub on_demand_admitted: bool,
}

impl Verdict {
    /// Derives the verdict from cell summaries.
    #[must_use]
    pub fn from_summaries(summaries: &[CellSummary], budget_ns: u64) -> Self {
        let mut worst_p99_ns = 0;
        let mut worst_cell = None;
        let mut failures = 0;
        for summary in summaries {
            failures += summary.failed;
            let hundred_way =
                summary.cell.method == Method::Ficlone && summary.cell.concurrency == 100;
            if hundred_way && summary.total.p99 >= worst_p99_ns {
                worst_p99_ns = summary.total.p99;
                worst_cell = Some(summary.cell.clone());
            }
        }
        let measured = worst_cell.is_some();
        Self {
            budget_ns,
            worst_p99_ns,
            worst_cell,
            failures,
            on_demand_admitted: measured && failures == 0 && worst_p99_ns < budget_ns,
        }
    }
}

/// Why the benchmark could not complete.
#[derive(Debug)]
pub enum BenchError {
    /// The output file could not be created or written.
    Output(io::Error),
    /// The storage profile probe rejected the directory.
    Profile(crate::profile::ProfileRejection),
    /// Template creation failed.
    Template(crate::template::TemplateError),
    /// Any other I/O failure with the step that failed.
    Io(&'static str, io::Error),
}

impl fmt::Display for BenchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Output(error) => write!(f, "output failed: {error}"),
            Self::Profile(rejection) => write!(f, "profile rejected: {rejection}"),
            Self::Template(error) => write!(f, "template failed: {error}"),
            Self::Io(step, error) => write!(f, "{step} failed: {error}"),
        }
    }
}

impl std::error::Error for BenchError {}

/// Runs the complete matrix and returns every cell summary plus the verdict.
///
/// # Errors
///
/// Returns the first failure that prevents the matrix from continuing; individual sample
/// failures are recorded, not returned.
pub fn run(config: &BenchConfig) -> Result<(Vec<CellSummary>, Verdict), BenchError> {
    let summaries = matrix::run(config)?;
    let verdict = Verdict::from_summaries(&summaries, config.budget_ns);
    Ok((summaries, verdict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cell::{Allocation, CacheState, Pressure, TemplateSize};

    fn summary(concurrency: usize, method: Method, p99: u64, failed: usize) -> CellSummary {
        let cell = Cell {
            template_size: TemplateSize::Gib1,
            allocation: Allocation::Sterile,
            cache: CacheState::Warm,
            concurrency,
            pressure: Pressure::None,
            method,
        };
        let stat = Percentiles {
            n: 1,
            min: p99,
            mean: p99,
            p50: p99,
            p95: p99,
            p99,
            max: p99,
        };
        CellSummary {
            cell,
            ok: 1,
            failed,
            total: stat,
            clone: stat,
            phases: record::PhaseSummary::default(),
        }
    }

    #[test]
    fn verdict_uses_the_worst_hundred_way_ficlone_cell() {
        let summaries = vec![
            summary(1, Method::Ficlone, 10, 0),
            summary(100, Method::Ficlone, 900_000, 0),
            summary(100, Method::Ficlone, 1_200_000, 0),
            summary(100, Method::CpReflink, 9_000_000, 0),
        ];
        let verdict = Verdict::from_summaries(&summaries, DEFAULT_BUDGET_NS);
        assert_eq!(verdict.worst_p99_ns, 1_200_000);
        assert!(!verdict.on_demand_admitted);
        let admitted = Verdict::from_summaries(&summaries[..2], DEFAULT_BUDGET_NS);
        assert!(admitted.on_demand_admitted);
        let failed =
            Verdict::from_summaries(&[summary(100, Method::Ficlone, 1, 1)], DEFAULT_BUDGET_NS);
        assert!(!failed.on_demand_admitted);
        let unmeasured =
            Verdict::from_summaries(&[summary(10, Method::Ficlone, 1, 0)], DEFAULT_BUDGET_NS);
        assert!(!unmeasured.on_demand_admitted);
    }
}
