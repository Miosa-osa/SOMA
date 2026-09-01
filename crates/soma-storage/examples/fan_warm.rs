//! Prepares a head root for concurrent launches: shard directories and one template fan.
//!
//! Both objects a cohort serializes on are removed here, off the launch path. Creating the
//! shards is instant. Warming the fan writes one template's bytes per copy and reads every copy
//! back to prove it is the template and owns its own extents, so it costs minutes for a 2 GiB
//! template on a rotational volume and is meant to run once per Generation, when the Generation
//! is prepared rather than when an Instance is launched.
//!
//! ```text
//! fan_warm --head-root /srv/soma/heads --template <overlay template or overlay.raw> \
//!          [--copies 4] [--shards 16]
//! ```
//!
//! The launch path reads what this writes through `SOMA_HEAD_DIR`, `SOMA_HEAD_SHARDS`,
//! `SOMA_TEMPLATE_FAN_DIR`, and `SOMA_TEMPLATE_COPIES`. A head root that was never warmed still
//! launches: the clone falls back to the template itself.

#![allow(clippy::print_stdout)]

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(item) = args.next() {
        if item == name {
            return args.next();
        }
    }
    None
}

fn count(name: &str, fallback: usize) -> NonZeroUsize {
    arg(name)
        .and_then(|value| value.parse().ok())
        .and_then(NonZeroUsize::new)
        .unwrap_or_else(|| NonZeroUsize::new(fallback).unwrap_or(NonZeroUsize::MIN))
}

fn main() -> ExitCode {
    let Some(head_root) = arg("--head-root").map(PathBuf::from) else {
        eprintln!("--head-root is required");
        return ExitCode::FAILURE;
    };
    let shards = count("--shards", soma_storage::DEFAULT_HEAD_SHARDS);
    if let Err(error) = soma_storage::create_shards(&head_root, shards) {
        eprintln!("shards: {error}");
        return ExitCode::FAILURE;
    }
    println!("shards {} under {}", shards.get(), head_root.display());

    let Some(template_path) = arg("--template").map(PathBuf::from) else {
        return ExitCode::SUCCESS;
    };
    let template = match std::fs::File::open(&template_path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("template: {error}");
            return ExitCode::FAILURE;
        }
    };
    let fan_root = arg("--fan-root").map_or_else(|| head_root.join("fan"), PathBuf::from);
    let copies = count("--copies", soma_storage::DEFAULT_TEMPLATE_COPIES);
    match soma_storage::warm(&template, &fan_root, copies) {
        Ok(report) => {
            println!(
                "fan {} copies {} written {} under {}",
                report.key,
                report.copies,
                report.written,
                fan_root.display()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("fan: {error}");
            ExitCode::FAILURE
        }
    }
}
