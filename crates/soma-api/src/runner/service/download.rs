//! `GET fs/download`: a file's raw bytes, streamed out of the guest as the client reads them.
//!
//! The file is stat'ed first, so a missing path or a directory is refused before any byte is
//! sent and the answer can state its length. The bytes then move one transfer at a time, each on
//! a facade lease of its own, so a slow reader holds no lease while it reads: what it holds is
//! the sandbox, which is not reaped while the download runs. A file that shrinks while it is read
//! ends the body short, and the transport aborts it instead of ending it, so a client never
//! takes part of a file for the whole of it; a file that grows is sent at the length it had.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use bytes::Bytes;
use soma::{FileKind, FileRefusal, InstanceId};

use crate::runner::{
    backend::Backend,
    files::{
        FileCall, OnFacade, Params, Stop,
        answer::attachment,
        error::FileError,
        guest::{read_range, stat},
        path::GuestPath,
        plan::required,
    },
    ids::SandboxId,
    sandboxes::Sandboxes,
};

use super::{
    Runner, RunnerRequest, RunnerResponse, Timing,
    files::{Hold, refused, stopped},
};

/// The most one facade call reads for a download.
const TRANSFER: usize = soma::MAX_FILE_BYTES;
/// Transfers read ahead of a slow client.
const READ_AHEAD: usize = 2;

impl Runner {
    pub(super) async fn download(
        &self,
        held: Hold<'_>,
        request: &RunnerRequest,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let path = match Params::read(request.query.as_deref(), b"")
            .map_err(|_| FileError::invalid_query())
            .and_then(|params| required(&params))
        {
            Ok((_, path)) => path,
            Err(error) => return refused(&error),
        };
        let (instance_id, checked) = (held.instance_id.clone(), path.clone());
        let size = match self
            .backend
            .call(move |facade| {
                stat(
                    &mut OnFacade {
                        facade,
                        instance_id: &instance_id,
                    },
                    &checked,
                )
            })
            .await
        {
            Ok((Ok(found), call)) if found.kind == FileKind::File => {
                timing.call = call;
                found.size
            }
            Ok((Ok(_), _)) => {
                return stopped(FileCall::Download, Stop::Refused(FileRefusal::WrongKind));
            }
            Ok((Err(stop), _)) => return stopped(FileCall::Download, stop),
            Err(_busy) => return RunnerResponse::runtime_busy(),
        };
        let (sender, stream) = tokio::sync::mpsc::channel(READ_AHEAD);
        let complete = Arc::new(AtomicBool::new(false));
        let reader = Reader {
            backend: self.backend.clone(),
            sandboxes: Arc::clone(&self.sandboxes),
            id: held.id.clone(),
            tenant_id: held.tenant_id.to_owned(),
            instance_id: held.instance_id,
            path: path.clone(),
            size,
        };
        let done = Arc::clone(&complete);
        tokio::spawn(async move {
            if reader.send(&sender).await {
                done.store(true, Ordering::Release);
            }
        });
        let mut response = RunnerResponse::new(200, Vec::new())
            .header("content-type", "application/octet-stream".to_owned())
            .header("content-disposition", attachment(path.file_name()))
            .header("content-length", size.to_string());
        response.stream = Some(stream);
        response.complete = Some(complete);
        response
    }
}

/// What the task sending one download's bytes owns.
struct Reader {
    backend: Backend,
    sandboxes: Arc<Sandboxes>,
    id: SandboxId,
    tenant_id: String,
    instance_id: InstanceId,
    path: GuestPath,
    size: u64,
}

impl Reader {
    /// Sends the file's bytes; `true` only when every one of them was sent.
    async fn send(&self, sender: &tokio::sync::mpsc::Sender<Bytes>) -> bool {
        // Held for the whole body, so the sandbox outlives the slowest reader.
        let Ok(_held) = self.sandboxes.hold(&self.id, &self.tenant_id) else {
            return false;
        };
        let mut offset = 0_u64;
        while offset < self.size {
            let length =
                usize::try_from(self.size - offset).map_or(TRANSFER, |left| left.min(TRANSFER));
            let (instance_id, path) = (self.instance_id.clone(), self.path.clone());
            let read = self
                .backend
                .call(move |facade| {
                    read_range(
                        &mut OnFacade {
                            facade,
                            instance_id: &instance_id,
                        },
                        &path,
                        offset,
                        length,
                    )
                })
                .await;
            let Ok((Ok((bytes, _end)), _)) = read else {
                return false;
            };
            // A read that returned nothing before the stated length means the file shrank.
            if bytes.is_empty() {
                return false;
            }
            offset = offset.saturating_add(bytes.len() as u64);
            if sender.send(Bytes::from(bytes)).await.is_err() {
                return false;
            }
        }
        true
    }
}
