mod app;
mod cli;
mod exit;
mod model;
mod render;
mod request;

use std::{env, ffi::OsString, io, process};

use clap::{Parser, error::ErrorKind};

use crate::{
    app::execute,
    cli::{Cli, OutputFormat},
    exit::ProcessExit,
    model::{FailureBody, Response},
};

fn main() {
    process::exit(run());
}

fn run() -> i32 {
    let arguments = env::args_os().collect::<Vec<_>>();
    let format = requested_format(&arguments);
    let cli = match Cli::try_parse_from(&arguments) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return if error.print().is_ok() {
                ProcessExit::Success.code()
            } else {
                ProcessExit::Software.code()
            };
        }
        Err(_) => return render_usage_error(format),
    };
    let format = cli.format;
    let execution = execute(cli);
    let rendered = render::render(
        &execution.response,
        format,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    );
    if rendered.is_err() {
        let _ = eprintln_bounded("soma: output failure");
        return ProcessExit::Software.code();
    }
    execution.exit.code()
}

fn render_usage_error(format: OutputFormat) -> i32 {
    let response = Response::failure("cli", FailureBody::usage());
    if render::render(
        &response,
        format,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    )
    .is_err()
    {
        let _ = eprintln_bounded("soma: output failure");
        return ProcessExit::Software.code();
    }
    ProcessExit::Usage.code()
}

fn requested_format(arguments: &[OsString]) -> OutputFormat {
    for (index, argument) in arguments.iter().enumerate() {
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if argument == "--" {
            break;
        }
        if argument == "--format=json"
            || (argument == "--format"
                && arguments.get(index + 1).and_then(|value| value.to_str()) == Some("json"))
        {
            return OutputFormat::Json;
        }
    }
    OutputFormat::Human
}

fn eprintln_bounded(message: &str) -> io::Result<()> {
    use io::Write as _;

    writeln!(io::stderr().lock(), "{message}")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::requested_format;
    use crate::cli::OutputFormat;

    #[test]
    fn recognizes_only_an_explicit_json_format_request() {
        let separate = [
            OsString::from("soma"),
            OsString::from("--format"),
            OsString::from("json"),
        ];
        let joined = [OsString::from("soma"), OsString::from("--format=json")];
        let human = [
            OsString::from("soma"),
            OsString::from("--format"),
            OsString::from("human"),
        ];

        assert_eq!(requested_format(&separate), OutputFormat::Json);
        assert_eq!(requested_format(&joined), OutputFormat::Json);
        assert_eq!(requested_format(&human), OutputFormat::Human);
    }
}
