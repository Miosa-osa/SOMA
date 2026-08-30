//! The line-oriented codec shared by the launcher tests and the `jail-probe` child.
//!
//! The probe stands in for the VMM in live tests: it reports what it can see and then executes
//! commands so the parent can prove containment.

use std::{error::Error, fmt};

/// What the probe observed after `execveat` from inside the jail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeReport {
    pub pid: i32,
    pub uid: u32,
    pub euid: u32,
    pub gid: u32,
    pub egid: u32,
    /// Whether every manifest slot had the expected kind and nothing else was open.
    pub table_sealed: bool,
    /// The first slot that failed verification, if any.
    pub first_bad_slot: Option<u32>,
    pub root: RootView,
}

/// What the probe saw of its root filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootView {
    /// Entries visible in `/`.
    pub entries: u32,
    /// Whether creating a file in `/` succeeded.
    pub writable: bool,
    pub proc_visible: bool,
    pub sys_visible: bool,
}

/// Typed codec failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportError {
    MissingField(&'static str),
    UnknownField,
    InvalidValue,
    UnknownCommand,
}

impl fmt::Display for ReportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(name) => write!(formatter, "report is missing {name}"),
            Self::UnknownField => write!(formatter, "report contains an unknown field"),
            Self::InvalidValue => write!(formatter, "report value is malformed"),
            Self::UnknownCommand => write!(formatter, "unknown probe command"),
        }
    }
}

impl Error for ReportError {}

const FIELDS: [&str; 11] = [
    "pid",
    "uid",
    "euid",
    "gid",
    "egid",
    "table_sealed",
    "first_bad_slot",
    "root_entries",
    "root_writable",
    "proc_visible",
    "sys_visible",
];

impl ProbeReport {
    #[must_use]
    pub fn encode(&self) -> String {
        let bad = self
            .first_bad_slot
            .map_or_else(|| "none".to_owned(), |slot| slot.to_string());
        format!(
            "pid={}\nuid={}\neuid={}\ngid={}\negid={}\ntable_sealed={}\nfirst_bad_slot={bad}\n\
             root_entries={}\nroot_writable={}\nproc_visible={}\nsys_visible={}\n",
            self.pid,
            self.uid,
            self.euid,
            self.gid,
            self.egid,
            self.table_sealed,
            self.root.entries,
            self.root.writable,
            self.root.proc_visible,
            self.root.sys_visible,
        )
    }

    /// Parses [`Self::encode`] output strictly: every field once, nothing unknown.
    ///
    /// # Errors
    ///
    /// Returns a [`ReportError`] for a missing, unknown, or malformed field.
    pub fn decode(text: &str) -> Result<Self, ReportError> {
        let mut values: [Option<&str>; FIELDS.len()] = [None; FIELDS.len()];
        for line in text.lines() {
            let (key, value) = line.split_once('=').ok_or(ReportError::InvalidValue)?;
            let index = FIELDS
                .iter()
                .position(|name| *name == key)
                .ok_or(ReportError::UnknownField)?;
            values[index] = Some(value);
        }
        for (index, name) in FIELDS.iter().enumerate() {
            if values[index].is_none() {
                return Err(ReportError::MissingField(name));
            }
        }
        let field = |index: usize| values[index].ok_or(ReportError::MissingField(FIELDS[index]));
        let number = |index: usize| {
            field(index)?
                .parse::<i64>()
                .map_err(|_| ReportError::InvalidValue)
        };
        let flag = |index: usize| match field(index)? {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(ReportError::InvalidValue),
        };
        let unsigned =
            |index: usize| u32::try_from(number(index)?).map_err(|_| ReportError::InvalidValue);
        let first_bad_slot = match field(6)? {
            "none" => None,
            other => Some(
                other
                    .parse::<u32>()
                    .map_err(|_| ReportError::InvalidValue)?,
            ),
        };
        Ok(Self {
            pid: i32::try_from(number(0)?).map_err(|_| ReportError::InvalidValue)?,
            uid: unsigned(1)?,
            euid: unsigned(2)?,
            gid: unsigned(3)?,
            egid: unsigned(4)?,
            table_sealed: flag(5)?,
            first_bad_slot,
            root: RootView {
                entries: unsigned(7)?,
                writable: flag(8)?,
                proc_visible: flag(9)?,
                sys_visible: flag(10)?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> ProbeReport {
        ProbeReport {
            pid: 1,
            uid: 60_001,
            euid: 60_001,
            gid: 60_001,
            egid: 60_001,
            table_sealed: true,
            first_bad_slot: None,
            root: RootView {
                entries: 0,
                writable: false,
                proc_visible: false,
                sys_visible: false,
            },
        }
    }

    #[test]
    fn report_round_trips() {
        let original = report();
        assert_eq!(ProbeReport::decode(&original.encode()), Ok(original));
        let bad = ProbeReport {
            first_bad_slot: Some(7),
            table_sealed: false,
            ..report()
        };
        assert_eq!(ProbeReport::decode(&bad.encode()), Ok(bad));
    }

    #[test]
    fn report_decoding_is_strict() {
        assert_eq!(
            ProbeReport::decode("pid=1\n"),
            Err(ReportError::MissingField("uid"))
        );
        let extra = format!("{}host=/tmp\n", report().encode());
        assert_eq!(ProbeReport::decode(&extra), Err(ReportError::UnknownField));
        let malformed = report().encode().replace("uid=60001", "uid=-1");
        assert_eq!(
            ProbeReport::decode(&malformed),
            Err(ReportError::InvalidValue)
        );
    }
}
