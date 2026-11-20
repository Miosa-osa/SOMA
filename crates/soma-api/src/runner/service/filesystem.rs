//! soma-api's own `/filesystem/write` and `/filesystem/read` on the runner, at any size up to
//! `MAX_FILESYSTEM_FILE_BYTES`.
//!
//! Every other `/filesystem/<op>` is forwarded to the loopback handler as before, and so are a
//! write whose body fits the loopback bound and a read whose file fits one transfer: their
//! answers are the loopback's own, byte for byte. A larger file never passes through the loopback
//! handler, whose single facade call moves at most four mebibytes. A write is read as it arrives,
//! its base64 decoded on the way through, and written one transfer at a time into a temporary
//! sibling that is renamed over the target once the body has ended; a read is answered as a
//! stream of transfers inside the same envelope the loopback renders. Both answer with the
//! documents the loopback answers with.

use tokio::sync::mpsc::Receiver;

use soma::{FileAnswer, InstanceId};

use crate::{
    envelope::ApiError,
    http::request::Method,
    runner::{
        body::{BodyChunk, StreamedBody},
        files::{
            OnFacade, Stop,
            guest::{Staged, kind_of, stat},
            path::GuestPath,
            write_body::WriteBody,
        },
        keys::Principal,
    },
};

use super::{
    MAX_BODY_BYTES, Runner, RunnerRequest, RunnerResponse, Timing,
    filesystem_answer::{Failed, answer, body_error, refused, validate},
    forward::unavailable,
    routing::Forward,
};

/// Decoded bytes gathered before one facade call writes them.
const TRANSFER: usize = soma::MAX_FILE_BYTES;

/// A sibling being filled, and the mode an existing target had, which the file keeps.
type Sibling = Option<(Staged, Option<u32>)>;

impl Runner {
    pub(super) async fn filesystem_write(
        &self,
        principal: &Principal,
        raw_id: &str,
        request: &RunnerRequest,
        timing: &mut Timing,
    ) -> RunnerResponse {
        let forward = Forward {
            id: raw_id,
            method: Method::Post,
            suffix: "/filesystem/write",
        };
        let Some(mut chunks) = request.streamed.as_ref().and_then(StreamedBody::take) else {
            return self
                .forward(principal, forward, &request.body, timing)
                .await;
        };
        // Up to the loopback bound the body is gathered and served exactly as before.
        let mut head = Vec::new();
        while head.len() <= MAX_BODY_BYTES {
            match chunks.recv().await {
                Some(Ok(chunk)) => head.extend_from_slice(&chunk),
                Some(Err(_fault)) => return refused(body_error()),
                None => return self.forward(principal, forward, &head, timing).await,
            }
        }
        let id = match self.addressed(raw_id) {
            Ok(id) => id,
            Err(response) => return *response,
        };
        match self.wait_for_launch(&id, &principal.key.tenant_id).await {
            Ok(waited) => timing.wait = waited,
            Err(response) => return *response,
        }
        // Held for the whole write, so the sandbox is never reaped under it.
        let _held = match self.sandboxes.hold(&id, &principal.key.tenant_id) {
            Ok(held) => held,
            Err(refused) => return unavailable(&refused),
        };
        let Some(instance_id) = id.instance_id() else {
            return RunnerResponse::platform(
                &crate::runner::public_wire::PlatformError::sandbox_not_found(),
            );
        };
        match self.write_large(&instance_id, head, chunks).await {
            Ok(bytes) => answer(instance_id, "write", &FileAnswer::Written { bytes }),
            Err(failed) => failed.answer(instance_id, "write"),
        }
    }

    /// Reads the rest of a large body into a staged sibling and publishes it; a write that does
    /// not complete discards the sibling and leaves the target as it was.
    async fn write_large(
        &self,
        instance_id: &InstanceId,
        head: Vec<u8>,
        chunks: Receiver<BodyChunk>,
    ) -> Result<u64, Failed> {
        let mut sibling: Sibling = None;
        let mut decoded = Vec::with_capacity(TRANSFER);
        let received = self
            .receive_write(instance_id, head, chunks, &mut sibling, &mut decoded)
            .await;
        let published = match (received, sibling.clone()) {
            (Ok(()), Some((mut file, mode))) => {
                match self.append(instance_id, &mut file, &mut decoded).await {
                    Ok(()) => {
                        sibling = Some((file.clone(), mode));
                        self.guest_call(instance_id, move |guest| file.publish(guest, mode))
                            .await
                    }
                    Err(failed) => Err(failed),
                }
            }
            (Ok(()), None) => Err(Failed::PathAfterContent),
            (Err(failed), _) => Err(failed),
        };
        if published.is_err()
            && let Some((file, _)) = sibling
        {
            let _discarded = self
                .guest_call(instance_id, move |guest| {
                    file.discard(guest);
                    Ok(())
                })
                .await;
        }
        published
    }

    /// Feeds the body to its reader, staging the file once its path is known.
    async fn receive_write(
        &self,
        instance_id: &InstanceId,
        head: Vec<u8>,
        mut chunks: Receiver<BodyChunk>,
        sibling: &mut Sibling,
        decoded: &mut Vec<u8>,
    ) -> Result<(), Failed> {
        let mut reader = WriteBody::new();
        self.take(instance_id, &mut reader, &head, sibling, decoded)
            .await?;
        while let Some(chunk) = chunks.recv().await {
            let chunk = chunk.map_err(|_| Failed::Interrupted)?;
            self.take(instance_id, &mut reader, &chunk, sibling, decoded)
                .await?;
        }
        let fields = reader
            .finish()
            .map_err(Failed::Body)?
            .ok_or(Failed::NoContent)?;
        validate(&fields, instance_id).map_err(Failed::Api)
    }

    async fn take(
        &self,
        instance_id: &InstanceId,
        reader: &mut WriteBody,
        chunk: &[u8],
        sibling: &mut Sibling,
        decoded: &mut Vec<u8>,
    ) -> Result<(), Failed> {
        reader.feed(chunk, decoded).map_err(Failed::Body)?;
        if sibling.is_none() && reader.content_started() {
            let Some(path) = reader.path() else {
                // The file cannot be staged before its destination is known.
                return Err(Failed::PathAfterContent);
            };
            soma::check_guest_path(path.as_bytes())
                .map_err(|rejected| Failed::Api(ApiError::invalid(rejected.message())))?;
            // The same rule as the line above, so this never refuses on its own.
            let target = GuestPath::carried(path).map_err(|_| {
                Failed::Api(ApiError::invalid(
                    "the path is not one the guest protocol carries",
                ))
            })?;
            *sibling = Some(self.begin(instance_id, target).await?);
        }
        if let Some((file, _)) = sibling.as_mut()
            && decoded.len() >= TRANSFER
        {
            self.append(instance_id, file, decoded).await?;
        }
        Ok(())
    }

    /// Names the sibling, and reads the mode an existing target has so the file keeps it.
    async fn begin(
        &self,
        instance_id: &InstanceId,
        target: GuestPath,
    ) -> Result<(Staged, Option<u32>), Failed> {
        self.guest_call(instance_id, move |guest| {
            let mode = match kind_of(guest, &target)? {
                Some(_) => Some(stat(guest, &target)?.mode),
                None => None,
            };
            Ok((Staged::begin(guest, target)?, mode))
        })
        .await
    }

    async fn append(
        &self,
        instance_id: &InstanceId,
        file: &mut Staged,
        decoded: &mut Vec<u8>,
    ) -> Result<(), Failed> {
        let pending = std::mem::replace(decoded, Vec::with_capacity(TRANSFER));
        let mut moved = file.clone();
        *file = self
            .guest_call(instance_id, move |guest| {
                moved.append(guest, &pending).map(|()| moved)
            })
            .await?;
        Ok(())
    }

    /// Runs one guest sequence on a facade lease of its own.
    pub(super) async fn guest_call<R: Send + 'static>(
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
