//! A claim-tracking reader over one parsed TOML table.
//!
//! Every accessor records the key it consumed.
//! [`TableReader::finish`] then rejects the first unclaimed key, which is how unknown fields
//! are reported with their full dotted path.

use std::collections::BTreeSet;

use toml::{Table, Value};

use crate::error::{BoundError, ParseError};

use super::MAX_STRING_BYTES;

pub(super) struct TableReader<'a> {
    table: &'a Table,
    path: String,
    claimed: BTreeSet<&'a str>,
}

impl<'a> TableReader<'a> {
    pub(super) fn new(table: &'a Table, path: &str) -> Self {
        Self {
            table,
            path: path.to_owned(),
            claimed: BTreeSet::new(),
        }
    }

    pub(super) fn field(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn claim(&mut self, key: &str) -> Option<&'a Value> {
        let (stored, value) = self.table.get_key_value(key)?;
        self.claimed.insert(stored.as_str());
        Some(value)
    }

    pub(super) fn string(&mut self, key: &str) -> Result<&'a str, ParseError> {
        self.optional_string(key)?
            .ok_or_else(|| ParseError::MissingField {
                field: self.field(key),
            })
    }

    pub(super) fn optional_string(&mut self, key: &str) -> Result<Option<&'a str>, ParseError> {
        let field = self.field(key);
        match self.claim(key) {
            None => Ok(None),
            Some(Value::String(value)) => bounded(&field, value).map(Some),
            Some(_) => Err(ParseError::WrongType {
                field,
                expected: "a string",
            }),
        }
    }

    pub(super) fn u64(&mut self, key: &str) -> Result<u64, ParseError> {
        self.optional_u64(key)?
            .ok_or_else(|| ParseError::MissingField {
                field: self.field(key),
            })
    }

    pub(super) fn optional_u64(&mut self, key: &str) -> Result<Option<u64>, ParseError> {
        let field = self.field(key);
        match self.claim(key) {
            None => Ok(None),
            Some(Value::Integer(value)) => {
                u64::try_from(*value)
                    .map(Some)
                    .map_err(|_| ParseError::WrongType {
                        field,
                        expected: "a non-negative integer",
                    })
            }
            Some(_) => Err(ParseError::WrongType {
                field,
                expected: "a non-negative integer",
            }),
        }
    }

    pub(super) fn u32(&mut self, key: &str) -> Result<u32, ParseError> {
        let field = self.field(key);
        u32::try_from(self.u64(key)?).map_err(|_| ParseError::WrongType {
            field,
            expected: "an integer between 0 and 4294967295",
        })
    }

    pub(super) fn optional_u32(&mut self, key: &str) -> Result<Option<u32>, ParseError> {
        let field = self.field(key);
        self.optional_u64(key)?
            .map(|value| {
                u32::try_from(value).map_err(|_| ParseError::WrongType {
                    field,
                    expected: "an integer between 0 and 4294967295",
                })
            })
            .transpose()
    }

    pub(super) fn optional_bool(&mut self, key: &str) -> Result<Option<bool>, ParseError> {
        let field = self.field(key);
        match self.claim(key) {
            None => Ok(None),
            Some(Value::Boolean(value)) => Ok(Some(*value)),
            Some(_) => Err(ParseError::WrongType {
                field,
                expected: "a boolean",
            }),
        }
    }

    /// Reads an optional array of strings, bounded by `maximum` entries.
    pub(super) fn strings(&mut self, key: &str, maximum: usize) -> Result<Vec<String>, ParseError> {
        let field = self.field(key);
        let Some(value) = self.claim(key) else {
            return Ok(Vec::new());
        };
        let Value::Array(items) = value else {
            return Err(ParseError::WrongType {
                field,
                expected: "an array of strings",
            });
        };
        if items.len() > maximum {
            return Err(BoundError::TooMany { field, maximum }.into());
        }
        let mut values = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let item_field = format!("{field}[{index}]");
            let Value::String(text) = item else {
                return Err(ParseError::WrongType {
                    field: item_field,
                    expected: "a string",
                });
            };
            values.push(bounded(&item_field, text)?.to_owned());
        }
        Ok(values)
    }

    pub(super) fn table(&mut self, key: &str) -> Result<TableReader<'a>, ParseError> {
        self.optional_table(key)?
            .ok_or_else(|| ParseError::MissingField {
                field: self.field(key),
            })
    }

    pub(super) fn optional_table(
        &mut self,
        key: &str,
    ) -> Result<Option<TableReader<'a>>, ParseError> {
        let field = self.field(key);
        match self.claim(key) {
            None => Ok(None),
            Some(Value::Table(table)) => Ok(Some(TableReader::new(table, &field))),
            Some(_) => Err(ParseError::WrongType {
                field,
                expected: "a table",
            }),
        }
    }

    /// Reads an optional array of tables, bounded by `maximum` entries.
    pub(super) fn tables(
        &mut self,
        key: &str,
        maximum: usize,
    ) -> Result<Vec<TableReader<'a>>, ParseError> {
        let field = self.field(key);
        let Some(value) = self.claim(key) else {
            return Ok(Vec::new());
        };
        let Value::Array(items) = value else {
            return Err(ParseError::WrongType {
                field,
                expected: "an array of tables",
            });
        };
        if items.len() > maximum {
            return Err(BoundError::TooMany { field, maximum }.into());
        }
        let mut readers = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            let item_field = format!("{field}[{index}]");
            let Value::Table(table) = item else {
                return Err(ParseError::WrongType {
                    field: item_field,
                    expected: "a table",
                });
            };
            readers.push(TableReader::new(table, &item_field));
        }
        Ok(readers)
    }

    /// Rejects the first key that no accessor claimed.
    pub(super) fn finish(self) -> Result<(), ParseError> {
        for key in self.table.keys() {
            if !self.claimed.contains(key.as_str()) {
                return Err(ParseError::UnknownField {
                    field: self.field(key),
                });
            }
        }
        Ok(())
    }
}

fn bounded<'a>(field: &str, value: &'a str) -> Result<&'a str, ParseError> {
    if value.len() > MAX_STRING_BYTES {
        return Err(BoundError::TooLong {
            field: field.to_owned(),
            maximum: MAX_STRING_BYTES,
        }
        .into());
    }
    if value.contains('\0') {
        return Err(BoundError::ForbiddenCharacter {
            field: field.to_owned(),
        }
        .into());
    }
    Ok(value)
}
