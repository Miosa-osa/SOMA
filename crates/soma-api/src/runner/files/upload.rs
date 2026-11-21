//! What one upload's form says, read as its multipart events arrive.
//!
//! The platform's upload takes a `file` part and a destination `path`, from the form or the
//! query, defaulting to the guest home. A path ending in `/` names a directory the file lands in
//! under its own name. Because the file is written as it arrives, the destination has to be known
//! when the file part begins: a `path` field that comes after the file part and names a different
//! destination is refused rather than honoured too late. Every client that builds the form with
//! the path first, as the platform's own SDKs do, is unaffected.

use super::{
    error::FileError,
    multipart::Event,
    params::Params,
    path::{GUEST_HOME, GuestPath},
};

/// The largest file an upload moves, as on the platform.
pub(crate) const MAX_UPLOAD_FILE_BYTES: u64 = 500 * 1024 * 1024;
/// How the bound above is named in a refusal.
pub(crate) const MAX_UPLOAD_LABEL: &str = "500 MiB";
/// The largest upload body: the file and room for the form around it.
pub(crate) const MAX_UPLOAD_BODY_BYTES: u64 = MAX_UPLOAD_FILE_BYTES + 1024 * 1024;
/// The longest value a form field other than the file may carry.
const MAX_FIELD_BYTES: usize = 8 * 1024;

/// Where an upload's file goes, decided when its part begins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Destination {
    pub(crate) target: GuestPath,
    /// The destination as the answer echoes it.
    pub(crate) echo: String,
    /// The file's own name, as the answer reports it.
    pub(crate) filename: String,
}

/// What one event asks of the upload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    Nothing,
    /// The file part began; its bytes go to this destination.
    Begin(Destination),
    /// Bytes of the file.
    Bytes(Vec<u8>),
}

#[derive(Debug)]
enum Part {
    None,
    Path(Vec<u8>),
    File,
    Ignored,
}

/// The form of one upload, as far as it has arrived.
#[derive(Debug)]
pub(crate) struct UploadForm {
    query_path: Option<String>,
    path_field: Option<String>,
    part: Part,
    /// The destination text the file was sent to, once its part began.
    sent_to: Option<String>,
    file_bytes: u64,
}

impl UploadForm {
    pub(crate) fn new(params: &Params) -> Self {
        Self {
            query_path: params.text("path").map(str::to_owned),
            path_field: None,
            part: Part::None,
            sent_to: None,
            file_bytes: 0,
        }
    }

    /// Takes one event.
    ///
    /// # Errors
    ///
    /// Returns the refusal the upload ends with: a destination the contract does not expose, a
    /// file larger than an upload moves, an oversized field, or a destination named too late.
    pub(crate) fn on(&mut self, event: Event) -> Result<Step, FileError> {
        match event {
            Event::Begin { name, filename } => match name.as_str() {
                "file" if self.sent_to.is_none() => {
                    self.part = Part::File;
                    self.begin(filename.as_deref()).map(Step::Begin)
                }
                "path" => {
                    self.part = Part::Path(Vec::new());
                    Ok(Step::Nothing)
                }
                _ => {
                    self.part = Part::Ignored;
                    Ok(Step::Nothing)
                }
            },
            Event::Data(bytes) => match &mut self.part {
                Part::File => {
                    self.file_bytes = self.file_bytes.saturating_add(bytes.len() as u64);
                    if self.file_bytes > MAX_UPLOAD_FILE_BYTES {
                        return Err(too_large());
                    }
                    Ok(Step::Bytes(bytes))
                }
                Part::Path(value) => {
                    value.extend_from_slice(&bytes);
                    if value.len() > MAX_FIELD_BYTES {
                        return Err(FileError::invalid_body("a form field is longer than 8 KiB"));
                    }
                    Ok(Step::Nothing)
                }
                Part::None | Part::Ignored => Ok(Step::Nothing),
            },
            Event::End => {
                if let Part::Path(value) = std::mem::replace(&mut self.part, Part::None) {
                    let value = String::from_utf8(value)
                        .map_err(|_| FileError::invalid_body("the path field is not UTF-8 text"))?;
                    if self.sent_to.as_ref().is_some_and(|sent| *sent != value) {
                        return Err(FileError::invalid_body(
                            "the path field must come before the file part",
                        ));
                    }
                    self.path_field = Some(value);
                }
                Ok(Step::Nothing)
            }
        }
    }

    /// The destination the file part goes to, checked as the platform checks it.
    fn begin(&mut self, filename: Option<&str>) -> Result<Destination, FileError> {
        let remote = self.remote();
        GuestPath::admit(&remote).map_err(|_| FileError::invalid_path())?;
        let filename = base_name(filename.unwrap_or("upload")).to_owned();
        let echo = if remote.ends_with('/') {
            format!("{remote}{filename}")
        } else {
            remote.clone()
        };
        let target = GuestPath::admit(&echo).map_err(|_| FileError::invalid_path())?;
        self.sent_to = Some(remote);
        Ok(Destination {
            target,
            echo,
            filename,
        })
    }

    /// The destination as the form names it so far: a form field over the query, as Phoenix
    /// merges them, and the guest home when neither names one.
    fn remote(&self) -> String {
        self.path_field
            .clone()
            .or_else(|| self.query_path.clone())
            .unwrap_or_else(|| format!("{GUEST_HOME}/"))
    }
}

/// The refusal for a file larger than an upload moves.
pub(crate) fn too_large() -> FileError {
    FileError::exceeds(MAX_UPLOAD_LABEL)
}

/// The last segment of a client's filename, as Elixir's `Path.basename/1` gives it.
fn base_name(filename: &str) -> &str {
    filename
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
