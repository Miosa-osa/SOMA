//! The execute encodings: the plain form every worker knows, and the form with a context.

use crate::Execute;

use super::super::field::{hex, tagged};

pub(super) fn encode_execute(execute: &Execute) -> String {
    if !execute.context().is_empty() {
        return encode_execute_in("execute_in", execute);
    }
    let limits = execute.limits();
    let mut text = format!(
        "execute {} {} {} {} {}",
        hex(execute.operation_id().as_bytes()),
        hex(execute.instance_id().as_bytes()),
        limits.timeout().get(),
        limits.output().get(),
        hex(execute.program().as_bytes()),
    );
    for argument in execute.arguments() {
        text.push(' ');
        text.push_str(&hex(argument.as_bytes()));
    }
    text
}

/// A command with a context, in a form of its own.
///
/// A worker built before contexts existed knows only `execute`, so it refuses this word and the
/// command never runs without the environment the caller asked for. The plain form above is
/// untouched, so an older worker keeps serving every command that has no context.
///
/// Layout: operation, instance, timeout, output, program, argument count and arguments,
/// environment count and name/value pairs, working directory, user, standard input. Every byte
/// string is `x`-prefixed hexadecimal, and an absent directory or user is `-`.
pub(super) fn encode_execute_in(word: &str, execute: &Execute) -> String {
    let limits = execute.limits();
    let context = execute.context();
    let mut text = format!(
        "{word} {} {} {} {} {} {}",
        hex(execute.operation_id().as_bytes()),
        hex(execute.instance_id().as_bytes()),
        limits.timeout().get(),
        limits.output().get(),
        tagged(execute.program().as_bytes()),
        execute.arguments().len(),
    );
    for argument in execute.arguments() {
        text.push(' ');
        text.push_str(&tagged(argument.as_bytes()));
    }
    text.push(' ');
    text.push_str(&context.environment.len().to_string());
    for (name, value) in &context.environment {
        text.push(' ');
        text.push_str(&tagged(name));
        text.push(' ');
        text.push_str(&tagged(value));
    }
    for optional in [&context.working_directory, &context.user] {
        text.push(' ');
        match optional {
            Some(bytes) => text.push_str(&tagged(bytes)),
            None => text.push('-'),
        }
    }
    text.push(' ');
    text.push_str(&tagged(&context.stdin));
    text
}
