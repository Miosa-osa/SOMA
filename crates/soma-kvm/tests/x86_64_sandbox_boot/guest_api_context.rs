//! The command half: the persisted environment, the command context, and streamed output.
//!
//! They live apart from the sequence so each file stays inside the source budget.

use super::*;

/// The persisted environment the control plane writes, and what a command sees.
pub(crate) fn persisted_environment<'a>(
    mut repaired: Repaired<'a>,
) -> Result<Repaired<'a>, String> {
    // 0600, rename), then seen by a direct command with no restart; a command's own value wins.
    let (next, _) = file(
        repaired,
        FileRequest::MakeDirectory {
            path: path("/etc/miosa"),
            parents: true,
        },
    )?;
    repaired = next;
    let env_file = b"FROM_FILE='from file'\nOVERRIDDEN='file value'\n";
    let (next, outcome) = repaired
        .write_range(
            RangeWrite {
                path: b"/etc/miosa/.identity.env.tmp",
                offset: 0,
                create: true,
                shorten: true,
                bytes: env_file,
            },
            4096,
        )
        .map_err(|error| format!("env temp: {error}"))?;
    repaired = next;
    check!(outcome == WholeFileWrite::Written, "env temp: {outcome:?}");
    let (next, outcome) = file(
        repaired,
        FileRequest::SetMode {
            path: path("/etc/miosa/.identity.env.tmp"),
            mode: 0o600,
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "env chmod: {outcome:?}");
    let (next, outcome) = file(
        repaired,
        FileRequest::Rename {
            path: path("/etc/miosa/.identity.env.tmp"),
            to: path("/etc/miosa/identity.env"),
        },
    )?;
    repaired = next;
    check!(outcome == FileOutcome::Done, "env rename: {outcome:?}");
    let context = CommandContext {
        environment: vec![(b"OVERRIDDEN".to_vec(), b"command value".to_vec())],
        working_directory: Some(b"/tmp".to_vec()),
        ..CommandContext::default()
    };
    let (next, env_out, _) = run(repaired, &["env"], context)?;
    repaired = next;
    eprintln!(
        "[guest-api] env sees: {}",
        env_out
            .lines()
            .filter(|line| line.contains("FILE") || line.contains("OVERRIDDEN"))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    check!(
        env_out.lines().any(|line| line == "FROM_FILE=from file"),
        "the persisted variable is missing"
    );
    check!(
        env_out
            .lines()
            .any(|line| line == "OVERRIDDEN=command value"),
        "the command's own value did not win"
    );
    check!(
        !env_out.contains("OVERRIDDEN=file value"),
        "the file value leaked past the command's"
    );

    // Update without a restart: the next command sees the new value.
    let (next, outcome) = repaired
        .write_range(
            RangeWrite {
                path: b"/etc/miosa/identity.env",
                offset: 0,
                create: false,
                shorten: true,
                bytes: b"FROM_FILE='updated'\n",
            },
            4096,
        )
        .map_err(|error| format!("env update: {error}"))?;
    repaired = next;
    check!(
        outcome == WholeFileWrite::Written,
        "env update: {outcome:?}"
    );
    let (next, env_out, _) = run(repaired, &["env"], CommandContext::default())?;
    repaired = next;
    check!(
        env_out.lines().any(|line| line == "FROM_FILE=updated"),
        "an update needed a restart"
    );
    Ok(repaired)
}

/// The working directory and the standard input a command context carries.
pub(crate) fn command_context<'a>(mut repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    // Working directory and stdin from the command context.
    let context = CommandContext {
        working_directory: Some(b"/tmp".to_vec()),
        stdin: b"typed input".to_vec(),
        ..CommandContext::default()
    };
    let (next, out, _) = run(repaired, &["sh", "-c", "pwd; cat"], context)?;
    repaired = next;
    check!(out == "/tmp\ntyped input", "cwd and stdin: {out:?}");
    Ok(repaired)
}

/// Output arrives while the command runs, not when it ends.
pub(crate) fn streaming_output<'a>(mut repaired: Repaired<'a>) -> Result<Repaired<'a>, String> {
    // Output arrives while the command runs, not when it ends.
    let command = GuestCommand::new(
        b"/bin/busybox".to_vec(),
        vec![
            b"sh".to_vec(),
            b"-c".to_vec(),
            b"echo first; sleep 2; echo second".to_vec(),
        ],
        30_000,
        1 << 20,
    )
    .map_err(|error| format!("streaming command: {error}"))?;
    let started = Instant::now();
    let mut seen: Vec<(Duration, String)> = Vec::new();
    let (next, outcome) = repaired
        .execute_streaming(operation(), command, |source, bytes| {
            if source == OutputSource::Stdout {
                seen.push((
                    started.elapsed(),
                    String::from_utf8_lossy(bytes).into_owned(),
                ));
            }
        })
        .map_err(|error| format!("execute_streaming: {error}"))?;
    repaired = next;
    eprintln!(
        "[guest-api] streamed chunks (elapsed, text): {seen:?}; final {:?}",
        outcome.status()
    );
    let first = seen
        .iter()
        .find(|(_, text)| text.contains("first"))
        .map(|(at, _)| *at);
    let second = seen
        .iter()
        .find(|(_, text)| text.contains("second"))
        .map(|(at, _)| *at);
    check!(
        matches!((first, second), (Some(first), Some(second)) if second.saturating_sub(first) >= Duration::from_millis(1500)),
        "output was not incremental: {seen:?}"
    );
    Ok(repaired)
}
