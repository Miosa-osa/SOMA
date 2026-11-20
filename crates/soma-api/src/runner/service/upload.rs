//! `POST fs/upload`: a `multipart/form-data` file, written into the guest as it arrives.
//!
//! The body is read chunk by chunk from the transport. Its file part's bytes are gathered into
//! transfers and each transfer is written into a temporary sibling of the destination on a
//! facade lease of its own, so the runner holds one transfer of the file at a time whatever its
//! size. Once the body has ended at its closing delimiter the sibling is given its mode and
//! renamed over the destination; anything that stops the upload first discards the sibling and
//! leaves the destination as it was.

use soma::InstanceId;
use tokio::sync::mpsc::Receiver;

use crate::runner::{
    body::{BodyChunk, BodyFault, StreamedBody},
    files::{
        FileCall, OnFacade, Params, Stop,
        answer::{WrittenFields, WrittenFile},
        error::FileError,
        guest::Staged,
        multipart::{Event, Multipart},
        upload::{Destination, Step, UploadForm, too_large},
    },
    public_wire::encode,
};

use super::{
    Runner, RunnerRequest, RunnerResponse, Timing,
    files::{Hold, refused, stopped},
};

/// The bytes gathered before one facade call writes them.
const TRANSFER: usize = soma::MAX_FILE_BYTES;
/// The mode an uploaded file ends with, as on the platform.
const UPLOADED_MODE: u32 = 0o644;

/// Why an upload stopped before its file was published.
enum Failed {
    Refused(FileError),
    Stopped(Stop),
    Busy,
}

/// The file part being written.
struct Writing {
    destination: Destination,
    staged: Staged,
    pending: Vec<u8>,
}

impl Runner {
    /// Each transfer is its own facade call, so the answer's `exec` timing is left at zero
    /// rather than reporting one of them as the whole.
    pub(super) async fn upload(
        &self,
        held: Hold<'_>,
        request: &RunnerRequest,
        _timing: &mut Timing,
    ) -> RunnerResponse {
        let Ok(params) = Params::read(request.query.as_deref(), b"") else {
            return refused(&FileError::invalid_query());
        };
        // The form is looked at before the body is taken, so a request that is not one is
        // refused without a byte of its body read.
        let Some(reader) = request
            .content_type
            .as_deref()
            .and_then(Multipart::for_content_type)
        else {
            return refused(&FileError::missing_file());
        };
        let Some(chunks) = request.streamed.as_ref().and_then(StreamedBody::take) else {
            return refused(&FileError::missing_file());
        };
        let mut writing = None;
        let outcome = self
            .receive(
                &held.instance_id,
                reader,
                UploadForm::new(&params),
                chunks,
                &mut writing,
            )
            .await;
        let published = match (outcome, writing) {
            (Ok(()), Some(file)) => self.publish(&held.instance_id, file).await,
            (Ok(()), None) => Err(Failed::Refused(FileError::missing_file())),
            (Err(failed), Some(file)) => {
                self.discard(&held.instance_id, file.staged).await;
                Err(failed)
            }
            (Err(failed), None) => Err(failed),
        };
        match published {
            Ok(body) => RunnerResponse::new(201, body),
            Err(Failed::Refused(error)) => refused(&error),
            Err(Failed::Stopped(stop)) => stopped(FileCall::Upload, stop),
            Err(Failed::Busy) => RunnerResponse::runtime_busy(),
        }
    }

    /// Reads the body to its closing delimiter, writing the file part as it arrives.
    async fn receive(
        &self,
        instance_id: &InstanceId,
        mut reader: Multipart,
        mut form: UploadForm,
        mut chunks: Receiver<BodyChunk>,
        writing: &mut Option<Writing>,
    ) -> Result<(), Failed> {
        let mut events = Vec::new();
        while let Some(chunk) = chunks.recv().await {
            let chunk = chunk.map_err(|fault| Failed::Refused(fault_error(fault)))?;
            reader
                .feed(&chunk, &mut events)
                .map_err(|_| Failed::Refused(malformed()))?;
            for event in events.drain(..) {
                self.take(instance_id, &mut form, event, writing).await?;
            }
        }
        if !reader.finished() {
            return Err(Failed::Refused(FileError::invalid_body(
                "the body ended before its closing multipart delimiter",
            )));
        }
        Ok(())
    }

    /// Applies one multipart event to the upload.
    async fn take(
        &self,
        instance_id: &InstanceId,
        form: &mut UploadForm,
        event: Event,
        writing: &mut Option<Writing>,
    ) -> Result<(), Failed> {
        match form.on(event).map_err(Failed::Refused)? {
            Step::Nothing => Ok(()),
            Step::Begin(destination) => {
                let target = destination.target.clone();
                let staged = self
                    .on_guest(instance_id, move |guest| Staged::begin(guest, target))
                    .await?;
                *writing = Some(Writing {
                    destination,
                    staged,
                    pending: Vec::with_capacity(TRANSFER),
                });
                Ok(())
            }
            Step::Bytes(bytes) => {
                let Some(file) = writing.as_mut() else {
                    return Ok(());
                };
                file.pending.extend_from_slice(&bytes);
                if file.pending.len() >= TRANSFER {
                    self.flush(instance_id, file).await?;
                }
                Ok(())
            }
        }
    }

    /// Writes the gathered bytes into the sibling.
    async fn flush(&self, instance_id: &InstanceId, file: &mut Writing) -> Result<(), Failed> {
        let pending = std::mem::replace(&mut file.pending, Vec::with_capacity(TRANSFER));
        let mut staged = file.staged.clone();
        file.staged = self
            .on_guest(instance_id, move |guest| {
                staged.append(guest, &pending).map(|()| staged)
            })
            .await?;
        Ok(())
    }

    /// Writes what is left, publishes the sibling, and builds the `201` body.
    async fn publish(
        &self,
        instance_id: &InstanceId,
        mut file: Writing,
    ) -> Result<Vec<u8>, Failed> {
        let written = match self.flush(instance_id, &mut file).await {
            Ok(()) => {
                let mut staged = file.staged.clone();
                self.on_guest(instance_id, move |guest| {
                    staged.publish(guest, Some(UPLOADED_MODE))
                })
                .await
            }
            Err(failed) => Err(failed),
        };
        let size_bytes = match written {
            Ok(size) => size,
            Err(failed) => {
                self.discard(instance_id, file.staged).await;
                return Err(failed);
            }
        };
        let destination = &file.destination;
        Ok(encode(&WrittenFile {
            file: WrittenFields {
                filename: &destination.filename,
                path: &destination.echo,
                size_bytes,
            },
            success: true,
        }))
    }

    async fn discard(&self, instance_id: &InstanceId, staged: Staged) {
        let _discarded = self
            .on_guest(instance_id, move |guest| {
                staged.discard(guest);
                Ok(())
            })
            .await;
    }

    /// Runs one guest sequence on a facade lease of its own.
    async fn on_guest<R: Send + 'static>(
        &self,
        instance_id: &InstanceId,
        work: impl FnOnce(&mut OnFacade<'_>) -> Result<R, Stop> + Send + 'static,
    ) -> Result<R, Failed> {
        let instance_id = instance_id.clone();
        match self
            .backend
            .call(move |facade| {
                work(&mut OnFacade {
                    facade,
                    instance_id: &instance_id,
                })
            })
            .await
        {
            Ok((Ok(value), _)) => Ok(value),
            Ok((Err(stop), _)) => Err(Failed::Stopped(stop)),
            Err(_busy) => Err(Failed::Busy),
        }
    }
}

fn fault_error(fault: BodyFault) -> FileError {
    match fault {
        BodyFault::TooLarge => too_large(),
        BodyFault::Stalled => FileError::upload_stalled(),
        BodyFault::Broken => FileError::invalid_body("the upload was interrupted"),
    }
}

fn malformed() -> FileError {
    FileError::invalid_body("the body is not a well-formed multipart/form-data body")
}
