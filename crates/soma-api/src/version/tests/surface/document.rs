//! The surface and versions the API reference document records.

use super::DOCUMENT;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

pub(super) fn digest(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    let hex = hash.iter().fold(String::new(), |mut hex, byte| {
        // Writing to a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
        hex
    });
    format!("sha256:{hex}")
}

/// The fenced block under `## Surface`.
pub(super) fn documented_surface() -> String {
    let section = DOCUMENT
        .split_once("\n## Surface\n")
        .expect("the document has a Surface section")
        .1;
    let block = section
        .split_once("```text\n")
        .expect("the Surface section has a text block")
        .1;
    block
        .split_once("```")
        .expect("the text block ends")
        .0
        .to_owned()
}

/// The rows of the version table: `| 1.0 | sha256:... | ... |`.
pub(super) fn documented_versions() -> Vec<(String, String)> {
    DOCUMENT
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            (cells.len() > 3 && cells[2].starts_with("sha256:"))
                .then(|| (cells[1].to_owned(), cells[2].to_owned()))
        })
        .collect()
}
