//! Measures the three create paths against machines this host really boots.
//!
//! Run it on a host with `/dev/kvm`, a prepared Generation for the reference it names, and a
//! scratch state root of its own:
//!
//! ```sh
//! SOMA_GENERATION_STORE=/srv/miosa/soma-qualification-fe3d01f/native-prepared \
//! SOMA_HEAD_DIR=/srv/miosa/soma-qualification-fe3d01f/native-heads \
//! SOMA_HEAD_SHARDS=16 SOMA_PREWARM_REFERENCE=node:22 SOMA_PREWARM_MEMORY_MIB=1024 \
//! SOMA_PREPARED_MACHINES=34 \
//! cargo run --release -p soma-api --example tti_measure -- /srv/miosa/tti-work/api/state
//! ```
//!
//! What it drives is [`soma_api::runner::Runner::handle`], the entry point the HTTP transport
//! calls once it has read a request; the wire above it is covered by the integration tests under
//! `tests/runner`. Every number it prints as server-side comes from the runner's own
//! `Server-Timing` and body fields, and each path's round trips are printed beside its totals so a
//! reader can add the network to one: on the wire a path costs one round trip per request it makes.

use std::{path::PathBuf, sync::Arc};

use soma_api::runner::{Backend, Journal, Runner};

mod harness;
mod measure;
mod report;

fn main() {
    // A hosted launch starts its machine host by re-entering this executable with
    // `machine-host`. Every binary that opens a hosted runtime has to answer here, before
    // anything else in it exists: without this the runtime would spawn a copy of this program
    // that measures instead of hosting, and every launch would fail.
    if std::env::args().nth(1).as_deref() == Some("machine-host") {
        std::process::exit(soma_local::host_machine(None));
    }
    bench();
}

#[tokio::main]
async fn bench() {
    let state_root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/soma-tti-measure".to_owned()),
    );
    std::fs::create_dir_all(&state_root).expect("the state root");
    if std::env::var("SOMA_PREPARED_MACHINES").is_err() {
        // Without this the host prepares one machine and every create but the first is a cold
        // boot, which would measure the wrong thing by an order of magnitude.
        println!("note: SOMA_PREPARED_MACHINES is unset, so most creates will be cold");
    }
    let journal_directory = state_root.join("journal");
    std::fs::create_dir_all(&journal_directory).expect("the journal directory");
    let config = Arc::new(harness::config(&journal_directory, harness::WORKERS));
    let journal = Journal::open(&config.journal.directory, &config.runner).expect("journal");
    let runner = Arc::new(Runner::new(
        Arc::clone(&config),
        Backend::new(harness::opener(&state_root, harness::WORKERS)),
        journal,
    ));
    harness::seed_keys(runner.keys());
    measure::all(&runner).await;
}
