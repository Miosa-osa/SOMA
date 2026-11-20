//! A permission mode as the platform's file contract states one, reduced to the bits a guest sets.
//!
//! The platform takes an octal string of three or four digits, an integer, or a simple symbolic
//! mode such as `u+x` or `go-w,a+r`, and runs `chmod` with it. The guest protocol sets the nine
//! permission bits and deliberately nothing else: a caller able to set set-user, set-group, or
//! sticky bits could turn a file it wrote into a privilege the guest's own users did not have.
//! So a mode naming one of those is refused here, by name, rather than silently narrowed.

use serde_json::Value;

/// Why a mode was not accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModeRefused {
    /// Not a mode the contract defines.
    Invalid,
    /// A set-user, set-group, or sticky bit, which the guest protocol does not set.
    SpecialBits,
}

/// A requested mode, before the path's current bits are known.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// The permission bits, exactly.
    Absolute(u32),
    /// Clauses applied in order to the path's current bits.
    Symbolic(Vec<Clause>),
}

/// One `who op perms` clause of a symbolic mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Clause {
    /// The bits of the classes named, as a mask over `0o777`.
    who: u32,
    op: u8,
    /// The `rwx` bits named, as one octal digit.
    permissions: u32,
    /// `X`: execute only for a directory, or where some class can already execute.
    search: bool,
}

impl Mode {
    /// Reads a mode the way the platform's `normalize_mode/1` accepts one.
    ///
    /// # Errors
    ///
    /// Returns why the value is not a mode this contract sets.
    pub(crate) fn parse(value: &Value) -> Result<Self, ModeRefused> {
        match value {
            Value::Number(number) => {
                let bits = number.as_u64().ok_or(ModeRefused::Invalid)?;
                absolute(u32::try_from(bits).map_err(|_| ModeRefused::Invalid)?)
            }
            Value::String(text) => {
                let text = text.trim();
                let octal = (3..=4).contains(&text.len())
                    && text.bytes().all(|b| (b'0'..=b'7').contains(&b));
                if octal {
                    absolute(u32::from_str_radix(text, 8).map_err(|_| ModeRefused::Invalid)?)
                } else {
                    symbolic(text)
                }
            }
            _ => Err(ModeRefused::Invalid),
        }
    }

    /// The bits this mode leaves on a path whose current bits and kind are given.
    pub(crate) fn apply(&self, current: u32, directory: bool) -> u32 {
        match self {
            Self::Absolute(bits) => *bits,
            Self::Symbolic(clauses) => clauses.iter().fold(current & 0o777, |bits, clause| {
                clause.apply(bits, directory)
            }),
        }
    }

    /// Whether the path's current bits are needed to know the result.
    pub(crate) const fn is_relative(&self) -> bool {
        matches!(self, Self::Symbolic(_))
    }

    /// The text the platform echoes for this mode.
    pub(crate) fn echo(&self, source: &Value) -> String {
        match (source, self) {
            (Value::String(text), _) => text.trim().to_owned(),
            (_, Self::Absolute(bits)) => format!("{bits:04o}"),
            (_, Self::Symbolic(_)) => String::new(),
        }
    }
}

fn absolute(bits: u32) -> Result<Mode, ModeRefused> {
    if bits > 0o7777 {
        return Err(ModeRefused::Invalid);
    }
    if bits & 0o7000 != 0 {
        return Err(ModeRefused::SpecialBits);
    }
    Ok(Mode::Absolute(bits))
}

fn symbolic(text: &str) -> Result<Mode, ModeRefused> {
    if text.is_empty() {
        return Err(ModeRefused::Invalid);
    }
    text.split(',')
        .map(clause)
        .collect::<Result<_, _>>()
        .map(Mode::Symbolic)
}

/// Reads one clause: one or more of `ugoa`, one of `+-=`, one or more of `rwxXst`.
fn clause(text: &str) -> Result<Clause, ModeRefused> {
    let split = text.find(['+', '-', '=']).ok_or(ModeRefused::Invalid)?;
    let (classes, rest) = text.split_at(split);
    let (op, permissions) = rest.split_at(1);
    if classes.is_empty() || permissions.is_empty() {
        return Err(ModeRefused::Invalid);
    }
    let mut who = 0;
    for class in classes.bytes() {
        who |= match class {
            b'u' => 0o700,
            b'g' => 0o070,
            b'o' => 0o007,
            b'a' => 0o777,
            _ => return Err(ModeRefused::Invalid),
        };
    }
    let mut clause = Clause {
        who,
        op: op.as_bytes()[0],
        permissions: 0,
        search: false,
    };
    for permission in permissions.bytes() {
        match permission {
            b'r' => clause.permissions |= 0o4,
            b'w' => clause.permissions |= 0o2,
            b'x' => clause.permissions |= 0o1,
            b'X' => clause.search = true,
            b's' | b't' => return Err(ModeRefused::SpecialBits),
            _ => return Err(ModeRefused::Invalid),
        }
    }
    Ok(clause)
}

impl Clause {
    fn apply(&self, bits: u32, directory: bool) -> u32 {
        let search = self.search && (directory || bits & 0o111 != 0);
        let digit = self.permissions | u32::from(search);
        // One digit repeated across the three classes, then narrowed to the classes named.
        let named = (digit * 0o111) & self.who;
        match self.op {
            b'+' => bits | named,
            b'-' => bits & !named,
            _ => (bits & !self.who) | named,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Mode, ModeRefused};

    fn result(value: &serde_json::Value, current: u32, directory: bool) -> u32 {
        Mode::parse(value)
            .expect("a mode")
            .apply(current, directory)
    }

    #[test]
    fn octal_text_and_integers_set_the_bits_exactly() {
        assert_eq!(result(&json!("0644"), 0o777, false), 0o644);
        assert_eq!(result(&json!("755"), 0, false), 0o755);
        assert_eq!(result(&json!(0o600), 0o777, false), 0o600);
        assert_eq!(result(&json!(" 0700 "), 0, false), 0o700);
    }

    #[test]
    fn symbolic_clauses_apply_in_order_to_the_current_bits() {
        assert_eq!(result(&json!("u+x"), 0o644, false), 0o744);
        assert_eq!(result(&json!("go-w"), 0o666, false), 0o644);
        assert_eq!(result(&json!("a=r"), 0o755, false), 0o444);
        assert_eq!(result(&json!("u=rwx,go=rx"), 0, false), 0o755);
        assert_eq!(
            result(&json!("a+X"), 0o644, false),
            0o644,
            "no class can execute a file"
        );
        assert_eq!(result(&json!("a+X"), 0o744, false), 0o755);
        assert_eq!(
            result(&json!("a+X"), 0o644, true),
            0o755,
            "a directory is searchable"
        );
    }

    #[test]
    fn special_bits_are_refused_by_name() {
        for value in [
            json!("4755"),
            json!("1777"),
            json!(0o2755),
            json!("u+s"),
            json!("+t"),
        ] {
            let refused = Mode::parse(&value);
            assert!(
                matches!(
                    refused,
                    Err(ModeRefused::SpecialBits | ModeRefused::Invalid)
                ),
                "{value}"
            );
        }
        assert_eq!(Mode::parse(&json!("4755")), Err(ModeRefused::SpecialBits));
        assert_eq!(Mode::parse(&json!("u+s")), Err(ModeRefused::SpecialBits));
    }

    #[test]
    fn what_is_not_a_mode_is_refused() {
        for value in [
            json!("88"),
            json!("07777777"),
            json!("rwx"),
            json!("u+"),
            json!("z+x"),
            json!(""),
            json!(-1),
            json!(0o10000),
            json!(true),
            json!(null),
        ] {
            assert_eq!(Mode::parse(&value), Err(ModeRefused::Invalid), "{value}");
        }
    }

    #[test]
    fn the_echo_is_the_text_the_caller_sent_or_four_octal_digits() {
        let source = json!(" 0644 ");
        assert_eq!(Mode::parse(&source).expect("mode").echo(&source), "0644");
        let source = json!(420);
        assert_eq!(Mode::parse(&source).expect("mode").echo(&source), "0644");
        let source = json!("u+x");
        assert_eq!(Mode::parse(&source).expect("mode").echo(&source), "u+x");
    }
}
