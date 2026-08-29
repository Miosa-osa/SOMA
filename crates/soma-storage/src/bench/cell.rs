//! The benchmark matrix dimensions and the cells they generate.

use serde::{Deserialize, Serialize};

/// Template logical size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateSize {
    /// 100 MiB.
    Mib100,
    /// 1 GiB.
    Gib1,
    /// 4 GiB.
    Gib4,
}

impl TemplateSize {
    /// Logical bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Mib100 => 100 * 1024 * 1024,
            Self::Gib1 => 1024 * 1024 * 1024,
            Self::Gib4 => 4 * 1024 * 1024 * 1024,
        }
    }

    /// Short label used in cell identifiers and class names.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mib100 => "100m",
            Self::Gib1 => "1g",
            Self::Gib4 => "4g",
        }
    }
}

/// How much of the template is allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Allocation {
    /// Only what `mke2fs` wrote; the rest is holes.
    Sterile,
    /// `fallocate` over the complete length after formatting.
    Preallocated,
    /// One 4 KiB unwritten extent every 128 KiB after formatting, so the extent count is
    /// about `bytes / 128 KiB` while the bytes stay identical to the sterile template.
    Fragmented,
}

impl Allocation {
    /// Short label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sterile => "sterile",
            Self::Preallocated => "prealloc",
            Self::Fragmented => "frag",
        }
    }
}

/// Host page cache state before each burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheState {
    /// No cache manipulation between bursts.
    Warm,
    /// `echo 3 > /proc/sys/vm/drop_caches` after `sync` before every burst.
    Cold,
}

impl CacheState {
    /// Short label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Warm => "warm",
            Self::Cold => "cold",
        }
    }
}

/// Additional filesystem pressure during the burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pressure {
    /// No extra pressure.
    None,
    /// The filesystem is filled to about 90 percent before the burst.
    FreeSpace,
    /// The same number of heads is unlinked concurrently with the burst.
    Cleanup,
}

impl Pressure {
    /// Short label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FreeSpace => "freespace",
            Self::Cleanup => "cleanup",
        }
    }
}

/// How the head is created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Method {
    /// In-process `FICLONE` through [`crate::clone::clone_head_timed`].
    Ficlone,
    /// One `cp --reflink=always` subprocess per head followed by the same syncs.
    CpReflink,
}

impl Method {
    /// Short label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ficlone => "ficlone",
            Self::CpReflink => "cp",
        }
    }
}

/// One point in the matrix.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cell {
    /// Template size.
    pub template_size: TemplateSize,
    /// Template allocation.
    pub allocation: Allocation,
    /// Cache state.
    pub cache: CacheState,
    /// Threads cloning in one burst.
    pub concurrency: usize,
    /// Extra pressure.
    pub pressure: Pressure,
    /// Creation method.
    pub method: Method,
}

impl Cell {
    /// Stable identifier used in records and tables.
    #[must_use]
    pub fn id(&self) -> String {
        format!(
            "{}-{}-{}-c{}-{}-{}",
            self.template_size.label(),
            self.allocation.label(),
            self.cache.label(),
            self.concurrency,
            self.pressure.label(),
            self.method.label()
        )
    }
}

const CONCURRENCY: [usize; 3] = [1, 10, 100];

/// The complete matrix, or the smoke subset without 4 GiB templates when `quick` is set.
#[must_use]
pub fn matrix(quick: bool) -> Vec<Cell> {
    let sizes: &[TemplateSize] = if quick {
        &[TemplateSize::Mib100, TemplateSize::Gib1]
    } else {
        &[TemplateSize::Mib100, TemplateSize::Gib1, TemplateSize::Gib4]
    };
    let allocations = [
        Allocation::Sterile,
        Allocation::Preallocated,
        Allocation::Fragmented,
    ];
    let mut cells = Vec::new();
    for &template_size in sizes {
        for allocation in allocations {
            for cache in [CacheState::Warm, CacheState::Cold] {
                for concurrency in CONCURRENCY {
                    cells.push(Cell {
                        template_size,
                        allocation,
                        cache,
                        concurrency,
                        pressure: Pressure::None,
                        method: Method::Ficlone,
                    });
                }
            }
        }
    }
    for allocation in allocations {
        for pressure in [Pressure::FreeSpace, Pressure::Cleanup] {
            cells.push(Cell {
                template_size: TemplateSize::Gib1,
                allocation,
                cache: CacheState::Warm,
                concurrency: 100,
                pressure,
                method: Method::Ficlone,
            });
        }
        for concurrency in CONCURRENCY {
            cells.push(Cell {
                template_size: TemplateSize::Gib1,
                allocation,
                cache: CacheState::Warm,
                concurrency,
                pressure: Pressure::None,
                method: Method::CpReflink,
            });
        }
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_matrix_has_every_required_cell_and_unique_ids() {
        let cells = matrix(false);
        assert_eq!(cells.len(), 3 * 3 * 2 * 3 + 3 * 2 + 3 * 3);
        let mut ids: Vec<String> = cells.iter().map(Cell::id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), cells.len());
        assert!(ids.contains(&"4g-prealloc-cold-c100-none-ficlone".to_owned()));
        assert!(ids.contains(&"1g-sterile-warm-c100-freespace-ficlone".to_owned()));
        assert!(ids.contains(&"1g-prealloc-warm-c100-cleanup-ficlone".to_owned()));
        assert!(ids.contains(&"1g-sterile-warm-c10-none-cp".to_owned()));
        assert!(ids.contains(&"4g-frag-cold-c100-none-ficlone".to_owned()));
        assert!(ids.contains(&"1g-frag-warm-c100-cleanup-ficlone".to_owned()));
        let hundred_way = cells
            .iter()
            .filter(|c| c.concurrency == 100 && c.method == Method::Ficlone)
            .count();
        assert_eq!(hundred_way, 24);
    }

    #[test]
    fn quick_matrix_drops_the_four_gib_templates() {
        let cells = matrix(true);
        assert!(cells.iter().all(|c| c.template_size != TemplateSize::Gib4));
        assert_eq!(cells.len(), 2 * 3 * 2 * 3 + 3 * 2 + 3 * 3);
        assert_eq!(TemplateSize::Gib4.bytes(), 4 * 1024 * 1024 * 1024);
    }
}
