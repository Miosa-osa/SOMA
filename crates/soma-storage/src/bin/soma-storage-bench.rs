//! Runs the XFS reflink benchmark matrix and prints the Markdown summary.
//!
//! ```text
//! soma-storage-bench --dir <xfs reflink mount> --out <samples.jsonl> [--samples N] [--quick]
//!                    [--budget-ns N]
//! ```
//!
//! The directory must sit on XFS with `reflink=1`; `templates/` and `heads/` are created
//! inside it and the output file is created exclusively.

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn parse(args: &[String]) -> Result<soma_storage::bench::BenchConfig, String> {
    use std::path::PathBuf;
    let mut dir = None;
    let mut out = None;
    let mut samples = 200usize;
    let mut quick = false;
    let mut budget_ns = soma_storage::bench::DEFAULT_BUDGET_NS;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dir" => dir = Some(PathBuf::from(iter.next().ok_or("--dir needs a value")?)),
            "--out" => out = Some(PathBuf::from(iter.next().ok_or("--out needs a value")?)),
            "--samples" => {
                samples = iter
                    .next()
                    .ok_or("--samples needs a value")?
                    .parse()
                    .map_err(|_| "--samples must be a positive integer".to_owned())?;
            }
            "--budget-ns" => {
                budget_ns = iter
                    .next()
                    .ok_or("--budget-ns needs a value")?
                    .parse()
                    .map_err(|_| "--budget-ns must be a positive integer".to_owned())?;
            }
            "--quick" => quick = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if samples == 0 {
        return Err("--samples must be at least 1".to_owned());
    }
    Ok(soma_storage::bench::BenchConfig {
        dir: dir.ok_or("--dir is required")?,
        out: out.ok_or("--out is required")?,
        samples,
        quick,
        budget_ns,
    })
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match parse(&args) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("soma-storage-bench: {message}");
            return ExitCode::from(2);
        }
    };
    match soma_storage::bench::run(&config) {
        Ok((summaries, verdict)) => {
            print!(
                "{}",
                soma_storage::bench::report::render(&summaries, &verdict)
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("soma-storage-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("soma-storage-bench: the XFS reflink benchmark runs only on Linux");
    ExitCode::from(2)
}
