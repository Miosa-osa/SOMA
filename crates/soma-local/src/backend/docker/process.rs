use std::{
    io::{Read, Result as IoResult},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) struct Result {
    pub(super) status: Option<ExitStatus>,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
    pub(super) timed_out: bool,
    pub(super) output_limited: bool,
}

pub(super) fn run(
    program: &str,
    args: &[String],
    timeout: Duration,
    output_limit: usize,
) -> std::io::Result<Result> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stdout_reader = thread::spawn(move || read_limited(stdout, output_limit));
    let stderr_reader = thread::spawn(move || read_limited(stderr, output_limit));
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let mut output_limited = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            timed_out = true;
            child.kill().ok();
            break Some(child.wait()?);
        }
        thread::sleep(Duration::from_millis(2));
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| std::io::Error::other("stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| std::io::Error::other("stderr reader panicked"))??;
    if stdout.len() > output_limit || stderr.len() > output_limit {
        output_limited = true;
    }
    Ok(Result {
        status,
        stdout: trim(stdout, output_limit),
        stderr: trim(stderr, output_limit),
        timed_out,
        output_limited,
    })
}

fn read_limited<R: Read>(reader: R, limit: usize) -> IoResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn trim(mut bytes: Vec<u8>, limit: usize) -> Vec<u8> {
    if bytes.len() > limit {
        bytes.truncate(limit);
    }
    bytes
}

pub(super) fn status_code(status: Option<ExitStatus>) -> i32 {
    status.and_then(|value| value.code()).unwrap_or(1)
}
