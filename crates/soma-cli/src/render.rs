use std::io::{self, Write};

use serde::Serialize;

use crate::{
    cli::OutputFormat,
    model::{
        CapabilityState, DoctorStatus, ENVELOPE_SCHEMA, FailureBody, MAX_OUTPUT_BYTES_USIZE,
        Response, ResultBody,
    },
};

const JSON_FIXED_OVERHEAD: usize = 384 * 1_024;
pub const MAX_JSON_BYTES: usize =
    base64_encoded_length(MAX_OUTPUT_BYTES_USIZE) + JSON_FIXED_OVERHEAD;

#[derive(Serialize)]
struct JsonEnvelope<'a> {
    schema: &'static str,
    command: &'static str,
    status: &'static str,
    result: Option<&'a ResultBody>,
    error: Option<&'a FailureBody>,
    receipt: Option<&'a soma::ExecutionReceipt>,
}

pub fn render(
    response: &Response,
    format: OutputFormat,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<()> {
    match format {
        OutputFormat::Human => render_human(response, stdout, stderr),
        OutputFormat::Json => render_json(response, stdout),
    }
}

fn render_json(response: &Response, stdout: &mut impl Write) -> io::Result<()> {
    if !response.output_is_within_declared_bound() {
        return Err(io::Error::other(
            "guest output exceeded its declared bound before encoding",
        ));
    }
    let envelope = JsonEnvelope {
        schema: ENVELOPE_SCHEMA,
        command: response.command(),
        status: response.status(),
        result: response.result(),
        error: response.error(),
        receipt: response.receipt(),
    };
    let mut bytes = serde_json::to_vec(&envelope).map_err(io::Error::other)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_JSON_BYTES {
        return Err(io::Error::other("JSON response exceeded its proven bound"));
    }
    stdout.write_all(&bytes)
}

fn render_human(
    response: &Response,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> io::Result<()> {
    match response.result() {
        Some(ResultBody::Command(report)) => {
            stdout.write_all(report.stdout.as_bytes())?;
            stderr.write_all(report.stderr.as_bytes())
        }
        Some(ResultBody::Version(report)) => {
            writeln!(stdout, "soma {}", report.version)?;
            writeln!(stdout, "contract: {}", report.envelope_schema)?;
            writeln!(
                stdout,
                "macos-development-lifecycle: {}",
                capability(report.macos_development_lifecycle)
            )?;
            writeln!(stdout, "native-kvm-lifecycle: unavailable")?;
            writeln!(stdout, "production-ready: no")
        }
        Some(ResultBody::Doctor(report)) => {
            writeln!(stdout, "doctor: {}", doctor_status(report.status))?;
            writeln!(stdout, "backend: {}", report.backend)?;
            writeln!(stdout, "reason: {}", report.reason)?;
            writeln!(stdout, "runtime-ready: {}", yes_no(report.runtime_ready))?;
            writeln!(stdout, "production-ready: no")
        }
        Some(ResultBody::Machine(report)) => {
            writeln!(stdout, "{}", report.instance_id.as_str())?;
            writeln!(stdout, "state: {}", report.state)
        }
        Some(ResultBody::Inspection(report)) => {
            writeln!(stdout, "{}", report.instance_id.as_str())?;
            writeln!(stdout, "state: {:?}", report.state)?;
            writeln!(stdout, "backend: {:?}", report.backend)
        }
        None => render_failure(response, stderr),
    }
}

fn render_failure(response: &Response, stderr: &mut impl Write) -> io::Result<()> {
    let error = response
        .error()
        .expect("a response without a result always has an error");
    writeln!(stderr, "soma: {}: {}", error.code, error.message)
}

const fn doctor_status(status: DoctorStatus) -> &'static str {
    match status {
        DoctorStatus::ProbePassed => "probe passed",
        DoctorStatus::ProbeFailed => "probe failed",
        DoctorStatus::Unsupported => "unsupported",
    }
}

const fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

const fn capability(state: CapabilityState) -> &'static str {
    match state {
        CapabilityState::Compiled => "compiled",
        CapabilityState::Unavailable => "unavailable",
    }
}

const fn base64_encoded_length(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::{
        cli::OutputFormat,
        model::{FailureBody, Response},
    };

    #[test]
    fn json_failure_has_a_stable_null_receipt() {
        let response = Response::failure(
            "run",
            FailureBody::new("backend_failure", "sandbox backend operation failed", true),
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        render(&response, OutputFormat::Json, &mut stdout, &mut stderr).expect("render JSON");

        let value: serde_json::Value = serde_json::from_slice(&stdout).expect("JSON");
        assert_eq!(value["schema"], "soma.cli.v1");
        assert_eq!(value["receipt"], serde_json::Value::Null);
        assert!(stderr.is_empty());
    }
}
