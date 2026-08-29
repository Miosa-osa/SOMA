//! Bounded diagnostic output to the guest console.
//!
//! Diagnostics never contain secrets, launch material, peer bytes, or command output.
//! Every line is truncated to a fixed length so a failure path cannot flood the console.

use std::fs::OpenOptions;
use std::io::Write;

const CONSOLE_PATH: &str = "/dev/console";
const PREFIX: &str = "soma-guest-agent: ";
const MAX_LINE_BYTES: usize = 240;

/// Writes one bounded line to the console, ignoring every write failure.
pub fn report(message: &str) {
    let line = bounded_line(message);
    if let Ok(mut console) = OpenOptions::new().write(true).open(CONSOLE_PATH) {
        let _ = console.write_all(line.as_bytes());
    }
}

/// Renders one prefixed, newline-terminated line no longer than the fixed bound.
#[must_use]
pub fn bounded_line(message: &str) -> String {
    let mut line = String::with_capacity(MAX_LINE_BYTES + 1);
    line.push_str(PREFIX);
    for character in message.chars() {
        if line.len() + character.len_utf8() > MAX_LINE_BYTES {
            break;
        }
        line.push(if character.is_control() {
            ' '
        } else {
            character
        });
    }
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_prefixed_terminated_and_bounded() {
        let line = bounded_line("boot ok");
        assert_eq!(line, "soma-guest-agent: boot ok\n");

        let long = bounded_line(&"x".repeat(1_000));
        assert_eq!(long.len(), MAX_LINE_BYTES + 1);
        assert!(long.ends_with('\n'));
    }

    #[test]
    fn control_characters_cannot_reach_the_console() {
        assert_eq!(bounded_line("a\x1b[2Jb\n"), "soma-guest-agent: a [2Jb \n");
    }
}
