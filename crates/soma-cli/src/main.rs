mod app;
mod cli;
mod exit;
mod model;
mod render;
mod request;

use std::{env, ffi::OsString, io, process};

use clap::{
    Parser,
    error::{ContextKind, ContextValue, ErrorKind},
};

use crate::{
    app::execute,
    cli::{Cli, OutputFormat, RootCommand},
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
        Err(error) => return render_usage_error(format, &error),
    };
    // A machine host is not a command that produces an envelope: it becomes the process that
    // holds one sandbox, so it takes the binary over before any response exists.
    if let RootCommand::MachineHost(arguments) = &cli.command {
        return soma_local::host_machine(Some(&arguments.socket));
    }
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

fn render_usage_error(format: OutputFormat, error: &clap::Error) -> i32 {
    // The envelope deliberately never echoes an argument value: an image reference or a guest
    // argument can be private, and a refusal must not be the thing that publishes it. The name
    // of the argument that failed is not a value, so naming it costs nothing and is the one
    // piece of information that turns "validation failed" into something an operator can act
    // on. Without it stderr is empty and the operator is left guessing.
    if let Some(name) = failing_argument(error) {
        let reason = usage_reason(error.kind());
        let _ = eprintln_bounded(&format!("soma: usage: {reason}: {name}; use --help"));
    }
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

/// The names of the arguments clap refused, when they are names rather than caller text.
///
/// An unrecognised argument is echoed back by clap as it was written, and what was written may
/// be `--flag=secret`. Only plain option and command names are reported, so no value can travel
/// out through this path.
/// What clap refused, as a phrase that names no caller value.
const fn usage_reason(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::MissingRequiredArgument | ErrorKind::MissingSubcommand => {
            "missing required argument"
        }
        ErrorKind::UnknownArgument | ErrorKind::InvalidSubcommand => "unrecognised argument",
        ErrorKind::InvalidValue | ErrorKind::ValueValidation => "invalid value for",
        ErrorKind::ArgumentConflict => "conflicting argument",
        ErrorKind::TooManyValues | ErrorKind::TooFewValues | ErrorKind::WrongNumberOfValues => {
            "wrong number of values for"
        }
        _ => "rejected argument",
    }
}

fn failing_argument(error: &clap::Error) -> Option<String> {
    let names: Vec<String> = match error.get(ContextKind::InvalidArg) {
        Some(ContextValue::String(name)) => safe_name(name).into_iter().collect(),
        Some(ContextValue::Strings(names)) => {
            names.iter().filter_map(|name| safe_name(name)).collect()
        }
        _ => Vec::new(),
    };
    (!names.is_empty()).then(|| names.join(", "))
}

/// One argument name, when what clap recorded is a name rather than something a caller typed.
///
/// clap writes an option that takes a value as `--name <PLACEHOLDER>`, so everything after the
/// first space is dropped, which also drops anything a caller could have smuggled into the rest
/// of the token. A token that is not a plain lowercase name is reported as nothing at all.
fn safe_name(recorded: &str) -> Option<String> {
    let name = recorded.split_whitespace().next()?;
    let bare = name.trim_start_matches('-');
    let plain = |character: char| {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '<' | '>' | '.')
    };
    (!bare.is_empty() && bare.len() <= 64 && bare.chars().all(plain)).then(|| name.to_owned())
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
