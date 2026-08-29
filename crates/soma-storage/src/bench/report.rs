//! Markdown rendering of cell summaries for the evidence document.

use std::fmt::Write;

use super::cell::{Method, Pressure};
use super::{CellSummary, Verdict};

/// Formats nanoseconds as microseconds with one decimal.
#[must_use]
pub fn micros(ns: u64) -> String {
    format!("{}.{}", ns / 1000, (ns % 1000) / 100)
}

fn table(out: &mut String, title: &str, rows: &[&CellSummary]) {
    if rows.is_empty() {
        return;
    }
    let _ = writeln!(out, "### {title}\n");
    let _ = writeln!(
        out,
        "| Cell | n | failed | total p50 us | total p95 us | total p99 us | total max us | clone p50 us | clone p99 us |"
    );
    let _ = writeln!(
        out,
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
    );
    for row in rows {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {} |",
            row.cell.id(),
            row.ok,
            row.failed,
            micros(row.total.p50),
            micros(row.total.p95),
            micros(row.total.p99),
            micros(row.total.max),
            micros(row.clone.p50),
            micros(row.clone.p99)
        );
    }
    out.push('\n');
}

fn phase_table(out: &mut String, rows: &[CellSummary]) {
    let _ = writeln!(out, "### Phase breakdown, p50 / p99 in microseconds\n");
    let _ = writeln!(
        out,
        "| Cell | create | clone | file fsync | dir fsync | verify | concurrent unlink |"
    );
    let _ = writeln!(out, "| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
    for row in rows {
        let p = &row.phases;
        let pair = |s: &super::stats::Percentiles| {
            if s.n == 0 {
                "-".to_owned()
            } else {
                format!("{} / {}", micros(s.p50), micros(s.p99))
            }
        };
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} | {} | {} |",
            row.cell.id(),
            pair(&p.create),
            pair(&p.clone),
            pair(&p.file_sync),
            pair(&p.dir_sync),
            pair(&p.verify),
            pair(&p.unlink)
        );
    }
    out.push('\n');
}

/// Renders every summary grouped by method and pressure, then the verdict.
#[must_use]
pub fn render(summaries: &[CellSummary], verdict: &Verdict) -> String {
    let mut out = String::new();
    let base: Vec<&CellSummary> = summaries
        .iter()
        .filter(|s| s.cell.method == Method::Ficlone && s.cell.pressure == Pressure::None)
        .collect();
    let pressured: Vec<&CellSummary> = summaries
        .iter()
        .filter(|s| s.cell.method == Method::Ficlone && s.cell.pressure != Pressure::None)
        .collect();
    let cp: Vec<&CellSummary> = summaries
        .iter()
        .filter(|s| s.cell.method == Method::CpReflink)
        .collect();
    table(&mut out, "In-process FICLONE, no extra pressure", &base);
    table(
        &mut out,
        "In-process FICLONE under free-space and cleanup pressure",
        &pressured,
    );
    table(&mut out, "cp --reflink=always subprocess comparison", &cp);
    phase_table(&mut out, summaries);
    let _ = writeln!(out, "### Verdict\n");
    let _ = writeln!(
        out,
        "Worst 100-way FICLONE p99: {} us in `{}` against a budget of {} us with {} failed samples.",
        micros(verdict.worst_p99_ns),
        verdict
            .worst_cell
            .as_ref()
            .map_or_else(|| "none".to_owned(), super::cell::Cell::id),
        micros(verdict.budget_ns),
        verdict.failures
    );
    let _ = writeln!(
        out,
        "{}",
        if verdict.on_demand_admitted {
            "On-demand cloning fits the budget on this host."
        } else {
            "On-demand cloning does not fit the budget; prepared sterile heads are mandatory."
        }
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::cell::{Allocation, CacheState, Cell, TemplateSize};
    use crate::bench::stats::Percentiles;

    #[test]
    fn micros_keeps_one_decimal() {
        assert_eq!(micros(0), "0.0");
        assert_eq!(micros(1_234), "1.2");
        assert_eq!(micros(1_000_000), "1000.0");
    }

    #[test]
    fn render_groups_cells_and_states_the_verdict() {
        let cell = Cell {
            template_size: TemplateSize::Mib100,
            allocation: Allocation::Sterile,
            cache: CacheState::Warm,
            concurrency: 100,
            pressure: Pressure::None,
            method: Method::Ficlone,
        };
        let stat = Percentiles {
            n: 100,
            min: 1,
            mean: 2,
            p50: 2_000,
            p95: 3_000,
            p99: 4_000,
            max: 5_000,
        };
        let summary = CellSummary {
            cell: cell.clone(),
            ok: 100,
            failed: 0,
            total: stat,
            clone: stat,
            phases: crate::bench::record::PhaseSummary::default(),
        };
        let verdict = Verdict::from_summaries(std::slice::from_ref(&summary), 1_000_000);
        let text = render(&[summary], &verdict);
        assert!(text.contains("### In-process FICLONE, no extra pressure"));
        assert!(text.contains("| `100m-sterile-warm-c100-none-ficlone` | 100 | 0 | 2.0 | 3.0 | 4.0 | 5.0 | 2.0 | 4.0 |"));
        assert!(!text.contains("cp --reflink=always subprocess comparison"));
        assert!(text.contains("fits the budget"));
        assert!(text.contains("### Phase breakdown"));
        assert!(text.contains("| `100m-sterile-warm-c100-none-ficlone` | - | - | - | - | - | - |"));
    }
}
