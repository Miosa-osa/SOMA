//! Drives the platform's file contract through the public runner against a machine this host
//! really boots, and fails on the first answer that is not the one the contract requires.
//!
//! Run it on a host with `/dev/kvm` and a prepared Generation for the image it names, with a
//! scratch state root of its own:
//!
//! ```sh
//! SOMA_GENERATION_STORE=<prepared store> SOMA_HEAD_DIR=<head directory> \
//! SOMA_ALLOW_UNCERTIFIED_GENERATION=1 \
//! cargo run --release -p soma-api --example runner_files -- <state root> [image] [memory_mib] [storage_mib]
//! ```
//!
//! The shape defaults to the facade's own default, the one `soma run` launches; a Generation
//! prepared at another shape is named by its memory and storage, because a snapshot restores only
//! at the shape it was captured at.
//!
//! `scripts/end-to-end-check.sh` runs it as its `runner_files` stage against the Generation it
//! compiled. What it drives is `Runner::handle`, the entry point the HTTP transport calls once it
//! has a request; the wire above it is covered by the integration tests under `tests/runner`.
//! It prints one line per call and, last, one summary line the end to end check records.

use std::{path::PathBuf, process::ExitCode, sync::Arc};

use soma_api::runner::{Backend, Journal, Runner};

mod flow;
mod harness;

fn main() -> ExitCode {
    // A hosted launch starts its machine host by re-entering this executable with
    // `machine-host`, so that has to be answered before anything else exists.
    if std::env::args().nth(1).as_deref() == Some("machine-host") {
        std::process::exit(soma_local::host_machine(None));
    }
    prove()
}

#[tokio::main]
async fn prove() -> ExitCode {
    let state_root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/soma-runner-files".to_owned()),
    );
    let image = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "node:22".to_owned());
    let mib = |index: usize, default: u64| {
        std::env::args()
            .nth(index)
            .map_or(Ok(default), |value| value.parse::<u64>())
            .expect("a size in MiB")
    };
    let shape = soma::MachineShape::new(
        soma::MachineShape::DEFAULT_VCPU_COUNT,
        mib(3, soma::MachineShape::DEFAULT_MEMORY_MIB),
        mib(4, soma::MachineShape::DEFAULT_STORAGE_MIB),
    )
    .expect("a valid shape");
    std::fs::create_dir_all(&state_root).expect("the state root");
    let journal_directory = state_root.join("journal");
    std::fs::create_dir_all(&journal_directory).expect("the journal directory");
    let config = Arc::new(harness::config(&journal_directory, &image, &shape));
    let journal = Journal::open(&config.journal.directory, &config.runner).expect("journal");
    let runner = Runner::new(
        Arc::clone(&config),
        Backend::new(harness::opener(&state_root)),
        journal,
    );
    harness::seed_keys(runner.keys());
    match flow::all(&runner).await {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("runner files proof failed: {failure}");
            ExitCode::FAILURE
        }
    }
}
