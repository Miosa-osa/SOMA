//! `/filesystem/read` of a file larger than one transfer, streamed inside the loopback's envelope.
//!
//! The first ranged read decides the path. A file that ends within it is answered as the loopback
//! answers it, from that one read; a guest without ranged reads is forwarded to the loopback
//! unchanged. A longer file is stat'ed for its length, and its envelope is rendered once with an
//! empty `content` and split at the content's data, so the bytes before and after the data are the
//! loopback's own rendering; the data between them is the base64 of one transfer after another,
//! each a multiple of three bytes so the pieces join into the base64 of the whole file. The answer
//! states its length, and a stream cut short is aborted by the transport rather than ended.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use soma::{FileAnswer, FileRefusal, InstanceId};

use crate::{
    envelope::{ApiError, Envelope, render},
    filesystem::{FilesystemBody, FilesystemReport},
    http::request::Method,
    route::FilesystemOperation,
    runner::{
        backend::Backend,
        files::{
            OnFacade, Stop,
            guest::{read_range, stat},
            path::GuestPath,
        },
        ids::SandboxId,
        keys::Principal,
        sandboxes::Sandboxes,
    },
};

use super::{
    Runner, RunnerRequest, RunnerResponse, Timing,
    filesystem_answer::{Failed, OPERATION, answer, refused},
    forward::unavailable,
    routing::Forward,
};

/// One transfer of a streamed read: a multiple of three bytes, so its base64 has no padding.
const READ_TRANSFER: usize = 3 * 1024 * 1024;
const _: () = assert!(READ_TRANSFER % 3 == 0 && READ_TRANSFER <= soma::MAX_FILE_BYTES);
/// Encoded transfers read ahead of a slow client.
const READ_AHEAD: usize = 2;
/// Where the empty content's data sits in the rendered envelope.
const EMPTY_CONTENT: &[u8] = br#""byte_length":0,"data":"""#;

impl Runner {
    pub(super) async fn filesystem_read(
        &self,
        principal: &Principal,
        raw_id: &str,
        request: &RunnerRequest,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let forward = Forward {
            id: raw_id,
            method: Method::Post,
            suffix: "/filesystem/read",
        };
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => timing.wait = waited,
            Err(response) => return *response,
        }
        let _held = match self.sandboxes.hold(&id, &principal.key.tenant_id) {
            Ok(held) => held,
            Err(refused) => return unavailable(&refused),
        };
        let Some(instance_id) = id.instance_id() else {
            return RunnerResponse::platform(
                &crate::runner::public_wire::PlatformError::sandbox_not_found(),
            );
        };
        let path = match read_path(&request.body, &instance_id) {
            Ok(path) => path,
            Err(error) => return refused(error),
        };
        let first = path.clone();
        let read = self
            .guest_call(&instance_id, move |guest| {
                read_range(guest, &first, 0, READ_TRANSFER)
            })
            .await;
        let (bytes, end) = match read {
            Ok(read) => read,
            // A guest without ranged reads reads one transfer whole, as it always did.
            Err(Failed::Stopped(Stop::Refused(FileRefusal::Unsupported))) => {
                return self
                    .forward(principal, forward, &request.body, timing)
                    .await;
            }
            Err(failed) => return failed.answer(instance_id, "read"),
        };
        if end {
            return answer(instance_id, "read", &FileAnswer::Read { bytes });
        }
        let sized = path.clone();
        let size = match self
            .guest_call(&instance_id, move |guest| stat(guest, &sized))
            .await
        {
            Ok(found) => found.size,
            Err(failed) => return failed.answer(instance_id, "read"),
        };
        if size > crate::runner::files::MAX_FILESYSTEM_FILE_BYTES {
            return answer(
                instance_id,
                "read",
                &FileAnswer::Refused(FileRefusal::TooLarge),
            );
        }
        let reader = Reader {
            backend: self.backend.clone(),
            sandboxes: Arc::clone(&self.sandboxes),
            id: id.clone(),
            tenant_id: principal.key.tenant_id.clone(),
            instance_id,
            path,
            size,
        };
        reader.answer(bytes)
    }
}

/// The path a read names, checked as the loopback route checks it.
fn read_path(body: &[u8], instance_id: &InstanceId) -> Result<GuestPath, ApiError> {
    let invalid = || ApiError::invalid("the request body is not a valid document for this route");
    let parsed: FilesystemBody = serde_json::from_slice(body).map_err(|_| invalid())?;
    let path = parsed.path.clone();
    parsed.into_facade(instance_id.clone(), FilesystemOperation::Read)?;
    GuestPath::carried(&path).map_err(|_| invalid())
}

/// What the task streaming one read owns.
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
    /// The `200` whose body is the envelope around the streamed data.
    fn answer(self, first: Vec<u8>) -> RunnerResponse {
        let (head, tail) = frame(&self.instance_id, self.size);
        let encoded = u64::try_from(base64_length(self.size)).unwrap_or(u64::MAX);
        let length = head.len() as u64 + encoded + tail.len() as u64;
        let (sender, stream) = tokio::sync::mpsc::channel(READ_AHEAD);
        let complete = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&complete);
        tokio::spawn(async move {
            if self.send(&sender, first, tail).await {
                done.store(true, Ordering::Release);
            }
        });
        let mut response =
            RunnerResponse::new(200, head).header("content-length", length.to_string());
        response.stream = Some(stream);
        response.complete = Some(complete);
        response
    }

    /// Sends the data and the envelope's tail; `true` only when every byte was sent.
    async fn send(
        &self,
        sender: &tokio::sync::mpsc::Sender<Bytes>,
        first: Vec<u8>,
        tail: Vec<u8>,
    ) -> bool {
        // Held for the whole body, so the sandbox outlives the slowest reader.
        let Ok(_held) = self.sandboxes.hold(&self.id, &self.tenant_id) else {
            return false;
        };
        let mut offset = first.len() as u64;
        if first.len() % 3 != 0 && offset < self.size {
            return false;
        }
        if sender
            .send(Bytes::from(STANDARD.encode(&first)))
            .await
            .is_err()
        {
            return false;
        }
        while offset < self.size {
            let left = usize::try_from(self.size - offset).unwrap_or(usize::MAX);
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
                        left.min(READ_TRANSFER),
                    )
                })
                .await;
            let Ok((Ok((bytes, _end)), _)) = read else {
                return false;
            };
            // A read short of a whole transfer before the stated length means the file shrank,
            // and base64 joined after it would not be the file's.
            if bytes.is_empty()
                || (bytes.len() % 3 != 0 && offset + (bytes.len() as u64) < self.size)
            {
                return false;
            }
            offset += bytes.len() as u64;
            if sender
                .send(Bytes::from(STANDARD.encode(&bytes)))
                .await
                .is_err()
            {
                return false;
            }
        }
        offset == self.size && sender.send(Bytes::from(tail)).await.is_ok()
    }
}

/// The envelope's bytes before and after the content's data, for a file of `size` bytes.
fn frame(instance_id: &InstanceId, size: u64) -> (Vec<u8>, Vec<u8>) {
    let report = FilesystemReport::new(
        instance_id.clone(),
        "read",
        &FileAnswer::Read { bytes: Vec::new() },
    );
    let rendered = render(&Envelope::success(OPERATION, &report, None));
    let at = rendered
        .windows(EMPTY_CONTENT.len())
        .position(|window| window == EMPTY_CONTENT)
        .expect("a read report renders its content's length and data together");
    let mut head = rendered[..at].to_vec();
    head.extend_from_slice(format!(r#""byte_length":{size},"data":""#).as_bytes());
    let tail = rendered[at + EMPTY_CONTENT.len() - 1..].to_vec();
    (head, tail)
}

fn base64_length(bytes: u64) -> u64 {
    bytes.div_ceil(3) * 4
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use soma::{FileAnswer, InstanceId};

    use super::frame;
    use crate::runner::service::filesystem_answer::OPERATION;
    use crate::{
        envelope::{Envelope, render},
        filesystem::FilesystemReport,
    };

    #[test]
    fn the_streamed_frame_around_the_data_is_the_loopback_rendering() {
        let instance = InstanceId::new("33333333333333333333333333333333").expect("id");
        let bytes: Vec<u8> = (0..=255).cycle().take(10_001).collect();
        let whole = render(&Envelope::success(
            OPERATION,
            &FilesystemReport::new(
                instance.clone(),
                "read",
                &FileAnswer::Read {
                    bytes: bytes.clone(),
                },
            ),
            None,
        ));
        let (head, tail) = frame(&instance, bytes.len() as u64);
        let mut streamed = head;
        streamed.extend_from_slice(STANDARD.encode(&bytes).as_bytes());
        streamed.extend_from_slice(&tail);
        assert_eq!(String::from_utf8(streamed), String::from_utf8(whole));
    }
}
