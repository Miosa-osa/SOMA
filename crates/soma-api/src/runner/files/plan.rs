//! One file call's arguments, checked the way the platform checks them, before the guest is asked.
//!
//! Each call is turned into a plan or a refusal here, in the order the platform's own actions
//! check their arguments, so a malformed call gets the answer it gets on the platform and never
//! reaches the sandbox. A plan carries the admitted guest paths, and the path text the caller
//! sent where the platform's answer echoes it.

use base64::{Engine as _, engine::general_purpose::STANDARD};

use super::{
    FileCall,
    error::FileError,
    guest::{Slice, Written},
    mode::{Mode, ModeRefused},
    params::Params,
    path::{GUEST_HOME, GuestPath},
};

/// The most a JSON read or write moves; anything larger goes through the streaming routes.
pub(crate) const MAX_JSON_FILE_BYTES: usize = 32 * 1024 * 1024;
/// How the bound above is named in a refusal.
pub(crate) const MAX_JSON_FILE_LABEL: &str = "32 MiB";
/// The most one ranged read returns, as on the platform.
const MAX_RANGE_BYTES: u64 = 524_288;

/// A checked file call, ready to perform.
#[derive(Debug)]
pub(crate) enum Plan {
    List {
        echo: String,
        path: GuestPath,
    },
    Read {
        echo: String,
        path: GuestPath,
    },
    ReadSlice {
        echo: String,
        path: GuestPath,
        slice: Slice,
    },
    Write {
        echo: String,
        path: GuestPath,
        contents: Vec<u8>,
        how: Written,
    },
    Stat {
        echo: String,
        path: GuestPath,
    },
    MakeDirectory {
        path: GuestPath,
    },
    Rename {
        from: GuestPath,
        to: GuestPath,
    },
    Delete {
        echo: String,
        path: GuestPath,
        recursive: bool,
    },
    Chmod {
        path: GuestPath,
        mode: Mode,
        echo: String,
    },
}

impl Plan {
    /// Checks one call's parameters.
    ///
    /// # Errors
    ///
    /// Returns the refusal the platform answers the same parameters with.
    pub(crate) fn of(call: FileCall, params: &Params) -> Result<Self, FileError> {
        match call {
            FileCall::List => {
                let echo = match params.get("path") {
                    None | Some(serde_json::Value::Null) => GUEST_HOME.to_owned(),
                    Some(serde_json::Value::String(text)) => text.clone(),
                    Some(_) => return Err(FileError::invalid_path()),
                };
                let path = admit(&echo)?;
                Ok(Self::List { echo, path })
            }
            FileCall::Read => {
                let (echo, path) = required(params)?;
                match slice(params)? {
                    Some(slice) => Ok(Self::ReadSlice { echo, path, slice }),
                    None => Ok(Self::Read { echo, path }),
                }
            }
            FileCall::Write => write(params),
            FileCall::Stat => required(params).map(|(echo, path)| Self::Stat { echo, path }),
            FileCall::MakeDirectory => {
                required(params).map(|(_, path)| Self::MakeDirectory { path })
            }
            FileCall::Delete => required(params).map(|(echo, path)| Self::Delete {
                echo,
                path,
                recursive: params.flag("recursive"),
            }),
            FileCall::Rename => {
                let (Some(from), Some(to)) = (params.text("from"), params.text("to")) else {
                    return Err(FileError::missing_paths());
                };
                Ok(Self::Rename {
                    from: admit(from)?,
                    to: admit(to)?,
                })
            }
            FileCall::Chmod => {
                let (_, path) = required(params)?;
                let source = match params.get("mode") {
                    None | Some(serde_json::Value::Null) => return Err(FileError::missing_mode()),
                    Some(source) => source,
                };
                let mode = Mode::parse(source).map_err(|refused| match refused {
                    ModeRefused::Invalid => FileError::invalid_mode(),
                    ModeRefused::SpecialBits => FileError::special_mode_bits(),
                })?;
                let echo = mode.echo(source);
                Ok(Self::Chmod { path, mode, echo })
            }
            FileCall::Unserved(route) => Err(FileError::unserved(route)),
        }
    }
}

/// The `path` field, present, non-empty, and exposed.
fn required(params: &Params) -> Result<(String, GuestPath), FileError> {
    let echo = params.text("path").ok_or_else(FileError::missing_path)?;
    Ok((echo.to_owned(), admit(echo)?))
}

fn admit(raw: &str) -> Result<GuestPath, FileError> {
    GuestPath::admit(raw).map_err(|_| FileError::invalid_path())
}

/// The part of a file a read asks for: `tail_bytes`, or `offset` and `length`, each capped.
fn slice(params: &Params) -> Result<Option<Slice>, FileError> {
    if !["tail_bytes", "offset", "length"]
        .iter()
        .any(|name| params.has(name))
    {
        return Ok(None);
    }
    let count = |name: &str, default: u64| match params.count(name) {
        None => Ok(default),
        Some(Ok(count)) => Ok(count),
        Some(Err(())) => Err(FileError::invalid_range()),
    };
    if params.has("tail_bytes") {
        return Ok(Some(Slice::Tail(
            count("tail_bytes", 0)?.min(MAX_RANGE_BYTES),
        )));
    }
    Ok(Some(Slice::Range {
        offset: count("offset", 0)?,
        length: count("length", MAX_RANGE_BYTES)?.min(MAX_RANGE_BYTES),
    }))
}

/// A JSON write, checked as the platform's `FileWrite.parse/1` and its action check one.
fn write(params: &Params) -> Result<Plan, FileError> {
    let contents = contents(params)?;
    let append = match params.get("append") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(append)) => *append,
        Some(_) => return Err(FileError::invalid_body("append must be true or false")),
    };
    let mode = match params.get("mode") {
        None | Some(serde_json::Value::Null) => None,
        Some(source @ serde_json::Value::String(text))
            if (3..=4).contains(&text.len())
                && text.bytes().all(|b| (b'0'..=b'7').contains(&b)) =>
        {
            match Mode::parse(source) {
                Ok(Mode::Absolute(bits)) => Some(bits),
                _ => {
                    return Err(FileError::invalid_body(
                        "the set-user, set-group and sticky bits cannot be set through the file API",
                    ));
                }
            }
        }
        Some(_) => return Err(FileError::invalid_body("mode must be octal such as 0644")),
    };
    let (echo, path) = required(params)?;
    if contents.len() > MAX_JSON_FILE_BYTES {
        return Err(FileError::too_large(MAX_JSON_FILE_LABEL, "fs/upload"));
    }
    Ok(Plan::Write {
        echo,
        path,
        contents,
        how: Written { append, mode },
    })
}

/// The bytes a write carries: `content_base64`, or `content` as text or as base64.
fn contents(params: &Params) -> Result<Vec<u8>, FileError> {
    if let Some(serde_json::Value::String(encoded)) = params.get("content_base64") {
        return decode(encoded, "content_base64");
    }
    let Some(serde_json::Value::String(content)) = params.get("content") else {
        return Err(FileError::invalid_body(
            "content or content_base64 is required",
        ));
    };
    match params.get("encoding") {
        None | Some(serde_json::Value::Null) => Ok(content.clone().into_bytes()),
        Some(serde_json::Value::String(encoding)) if encoding == "utf8" || encoding == "utf-8" => {
            Ok(content.clone().into_bytes())
        }
        Some(serde_json::Value::String(encoding)) if encoding == "base64" => {
            decode(content, "content")
        }
        Some(_) => Err(FileError::invalid_body("encoding must be utf8 or base64")),
    }
}

fn decode(encoded: &str, field: &str) -> Result<Vec<u8>, FileError> {
    STANDARD
        .decode(encoded)
        .map_err(|_| FileError::invalid_body(&format!("{field} is not valid base64")))
}
