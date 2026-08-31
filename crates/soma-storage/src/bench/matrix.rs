//! Executes every cell of the matrix and writes the records.

use std::fs::File;
use std::os::fd::AsFd;

use super::burst::{self, HeadsDir};
use super::cell::{self, CacheState, Cell, Pressure};
use super::record::{Record, Writer};
use super::stats::Percentiles;
use super::templates::{self, BenchTemplate};
use super::{BenchConfig, BenchError, CellSummary};
use crate::clone;
use crate::head::{HeadName, HeadToken};
use crate::profile::StorageProfile;

/// Free space left when the free-space pressure cell runs.
const PRESSURE_FREE_PERCENT: u64 = 10;

/// Runs every cell and returns the summaries in matrix order.
///
/// # Errors
///
/// Returns the first failure that prevents the matrix from continuing.
pub fn run(config: &BenchConfig) -> Result<Vec<CellSummary>, BenchError> {
    let templates_dir = config.dir.join("templates");
    let heads_path = config.dir.join("heads");
    std::fs::create_dir_all(&templates_dir)
        .map_err(|e| BenchError::Io("create templates dir", e))?;
    std::fs::create_dir_all(&heads_path).map_err(|e| BenchError::Io("create heads dir", e))?;
    let heads = HeadsDir {
        file: File::open(&heads_path).map_err(|e| BenchError::Io("open heads dir", e))?,
        path: heads_path,
    };
    let profile = StorageProfile::probe(heads.file.as_fd()).map_err(BenchError::Profile)?;
    let identity = super::identity::RunIdentity::gather(&heads.path, &profile, config.samples)
        .map_err(|e| BenchError::Io("gather identity", e))?;
    let mut writer = Writer::create(&config.out).map_err(BenchError::Output)?;
    writer
        .write(&Record::Identity(identity))
        .map_err(BenchError::Output)?;

    let sizes: Vec<cell::TemplateSize> = {
        let mut sizes: Vec<_> = cell::matrix(config.quick)
            .iter()
            .map(|c| c.template_size)
            .collect();
        sizes.dedup();
        sizes.sort_by_key(|size| size.bytes());
        sizes.dedup();
        sizes
    };
    let templates = templates::prepare(&templates_dir, &sizes)?;
    for template in &templates {
        writer
            .write(&Record::Template(template.record.clone()))
            .map_err(BenchError::Output)?;
    }

    let mut counter = 0u128;
    let mut summaries = Vec::new();
    for cell in cell::matrix(config.quick) {
        let template = templates
            .iter()
            .find(|t| t.size == cell.template_size && t.allocation == cell.allocation)
            .ok_or_else(|| BenchError::Io("template lookup", std::io::Error::other(cell.id())))?;
        let summary = run_cell(config, &cell, template, &heads, &mut writer, &mut counter)?;
        eprintln!(
            "{:<40} n={:<4} failed={:<3} p50={:>9} p99={:>9} max={:>9} ns",
            summary.cell.id(),
            summary.ok,
            summary.failed,
            summary.total.p50,
            summary.total.p99,
            summary.total.max
        );
        summaries.push(summary);
    }
    writer.finish().map_err(BenchError::Output)?;
    Ok(summaries)
}

fn next_names(counter: &mut u128, count: usize) -> Vec<HeadName> {
    (0..count)
        .map(|_| {
            *counter += 1;
            HeadToken::new(counter.to_be_bytes())
                .expect("counter is never zero")
                .head_name()
        })
        .collect()
}

fn run_cell(
    config: &BenchConfig,
    cell: &Cell,
    template: &BenchTemplate,
    heads: &HeadsDir,
    writer: &mut Writer,
    counter: &mut u128,
) -> Result<CellSummary, BenchError> {
    let bursts = config.samples.div_ceil(cell.concurrency).max(1);
    let filler = if cell.pressure == Pressure::FreeSpace {
        Some(
            super::pressure::fill(&heads.path, heads.file.as_fd(), PRESSURE_FREE_PERCENT)
                .map_err(|e| BenchError::Io("fill filesystem", e))?,
        )
    } else {
        None
    };
    let mut totals = Vec::new();
    let mut clones = Vec::new();
    let mut all_samples = Vec::new();
    let mut unlink_totals = Vec::new();
    let mut failed = 0;
    for burst in 0..bursts {
        let names = next_names(counter, cell.concurrency);
        let victims = if cell.pressure == Pressure::Cleanup {
            let victims = next_names(counter, cell.concurrency);
            for victim in &victims {
                let head = clone::clone_head(
                    template.file.as_fd(),
                    heads.file.as_fd(),
                    victim,
                    clone::Durability::Persisted,
                )
                .map_err(|e| {
                    BenchError::Io("precreate victim", std::io::Error::other(e.to_string()))
                })?;
                drop(head);
            }
            victims
        } else {
            Vec::new()
        };
        if cell.cache == CacheState::Cold {
            burst::drop_caches().map_err(|e| BenchError::Io("drop caches", e))?;
        }
        let outcome = burst::run_burst(cell.method, template, heads, &names, &victims, burst);
        for sample in &outcome.samples {
            if sample.ok {
                totals.push(sample.total_ns);
                clones.push(sample.clone_ns);
            } else {
                failed += 1;
            }
            writer
                .write(&Record::Sample {
                    cell: cell.id(),
                    dimensions: cell.clone(),
                    sample: sample.clone(),
                })
                .map_err(BenchError::Output)?;
            all_samples.push(sample.clone());
        }
        for unlink in &outcome.unlinks {
            if unlink.ok {
                unlink_totals.push(unlink.total_ns);
            }
            writer
                .write(&Record::Unlink {
                    cell: cell.id(),
                    burst,
                    thread: unlink.thread,
                    ok: unlink.ok,
                    total_ns: unlink.total_ns,
                })
                .map_err(BenchError::Output)?;
        }
        burst::cleanup(heads.file.as_fd(), &names)
            .map_err(|e| BenchError::Io("cleanup heads", e))?;
        burst::cleanup(heads.file.as_fd(), &victims)
            .map_err(|e| BenchError::Io("cleanup victims", e))?;
    }
    if let Some(filler) = filler {
        filler
            .remove(&heads.path)
            .map_err(|e| BenchError::Io("remove filler", e))?;
    }
    let summary = CellSummary {
        cell: cell.clone(),
        ok: totals.len(),
        failed,
        total: Percentiles::of(&totals),
        clone: Percentiles::of(&clones),
        phases: super::record::PhaseSummary::of(&all_samples, &unlink_totals),
    };
    writer
        .write(&Record::Summary {
            cell: cell.id(),
            dimensions: cell.clone(),
            ok: summary.ok,
            failed,
            total: summary.total,
            clone: summary.clone,
            phases: summary.phases,
        })
        .map_err(BenchError::Output)?;
    Ok(summary)
}
