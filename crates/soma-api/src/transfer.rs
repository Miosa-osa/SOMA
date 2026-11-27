//! Checkpoint transfer to and from off-host storage (ADR 0077), on the loopback host listener.
//!
//! Export streams one artifact of a published capture as a raw body, and receive writes one raw
//! body into a private import stage; both are served by the connection because their bodies are
//! far larger than any JSON body this service reads. Commit and abort are ordinary JSON routes
//! (`handler::transfer`). Every transfer route answers the platform tenant only and any other
//! caller as if the route did not exist.

use std::{io::BufReader, net::TcpStream};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::io::{Read as _, Write as _};

use crate::{
    envelope::{ApiError, failure_body},
    facade::SandboxFacade,
    http::{request::Request, response::Response},
    route::{TransferArtifact, TransferRoute},
    tenant::{TENANT_HEADER, identify},
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::{
    envelope::{Envelope, render},
    version::{API_VERSION, API_VERSION_HEADER},
};

mod refusal;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use refusal::import_refusal;
pub use refusal::refusal;

/// The largest artifact one receive accepts: the largest durable volume class a host offers.
pub const MAX_ARTIFACT_BYTES: u64 = 1 << 40;

/// Refuses every caller except the platform, as if the route did not exist.
///
/// # Errors
///
/// The identity refusal, or a 404 for a tenant that is not the platform.
pub fn platform_only(request: &Request) -> Result<(), ApiError> {
    if identify(request.header(TENANT_HEADER))?.is_platform() {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}

/// Whether the connection comes from this host: the transfer routes serve no other peer, whatever
/// identity a request names.
#[must_use]
pub fn from_loopback(stream: &TcpStream) -> bool {
    stream
        .peer_addr()
        .is_ok_and(|address| address.ip().is_loopback())
}

fn answer(stream: &TcpStream, operation: &'static str, outcome: Result<Response, ApiError>) {
    let response = outcome
        .unwrap_or_else(|error| Response::new(error.status(), failure_body(operation, error)));
    let _ignored = response.write_to(&mut &*stream);
}

/// The export request: who owns the capture, its full receipt, and the artifact wanted.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportBody {
    pub owner: soma::MachineName,
    pub receipt: soma_local::CheckpointReceipt,
    pub artifact: String,
}

/// Streams one artifact of a published capture, or refuses before any byte of it is sent.
///
/// The body is exactly the artifact; `soma-artifact-digest` names the digest the receipt holds
/// for it. A read or digest failure part way closes the connection short of `content-length`,
/// so a caller that checks the length and the digest never keeps a damaged artifact.
pub fn serve_export<M, F>(stream: &TcpStream, request: &Request, open_facade: &M)
where
    M: Fn() -> Result<F, ApiError>,
    F: SandboxFacade,
{
    let operation = TransferRoute::Export.operation();
    if let Err(error) = platform_only(request) {
        return answer(stream, operation, Err(error));
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let opened = open_export(request, open_facade);
        let (mut export, artifact) = match opened {
            Ok(opened) => opened,
            Err(error) => return answer(stream, operation, Err(error)),
        };
        let Some((length, digest)) = export.describe(artifact) else {
            return answer(
                stream,
                operation,
                Err(refusal(soma_local::CheckpointFailure::NotFound)),
            );
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\ncontent-length: \
             {length}\r\ncache-control: no-store\r\nconnection: close\r\nsoma-artifact-digest: \
             {digest}\r\n{API_VERSION_HEADER}: {API_VERSION}\r\n\r\n"
        );
        let mut writer = std::io::BufWriter::with_capacity(1024 * 1024, stream);
        if writer.write_all(head.as_bytes()).is_err() {
            return;
        }
        if export.write_artifact(artifact, &mut writer).is_err() || writer.flush().is_err() {
            eprintln!("soma-api: {operation} stopped short; the caller sees a short body");
            let _ignored = stream.shutdown(std::net::Shutdown::Both);
        }
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = open_facade;
        answer(
            stream,
            operation,
            Err(refusal(soma_local::CheckpointFailure::Unsupported)),
        );
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn open_export<M, F>(
    request: &Request,
    open_facade: &M,
) -> Result<(soma_local::CheckpointExport, soma_local::CheckpointArtifact), ApiError>
where
    M: Fn() -> Result<F, ApiError>,
    F: SandboxFacade,
{
    let body: ExportBody = serde_json::from_slice(&request.body).map_err(|_| {
        ApiError::invalid("the request body is not a valid document for this route")
    })?;
    let artifact = local_artifact(TransferArtifact::parse(&body.artifact)?);
    // The facade is held only to authorize and open the capture: the open descriptors pin the
    // files, so the stream needs no state-store handle however long it takes.
    let mut facade = open_facade()?;
    let export = facade
        .open_checkpoint_export(&body.owner, &body.receipt)
        .map_err(refusal)?;
    Ok((export, artifact))
}

/// The artifact of the fixed checkpoint format a transfer path names.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[must_use]
pub const fn local_artifact(artifact: TransferArtifact) -> soma_local::CheckpointArtifact {
    match artifact {
        TransferArtifact::State => soma_local::CheckpointArtifact::State,
        TransferArtifact::Memory => soma_local::CheckpointArtifact::Memory,
        TransferArtifact::Overlay => soma_local::CheckpointArtifact::Overlay,
        TransferArtifact::Workspace => soma_local::CheckpointArtifact::Workspace,
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[derive(serde::Serialize)]
struct Received<'a> {
    import_id: &'a str,
    artifact: &'static str,
    bytes: u64,
    digest: String,
}

/// Receives one raw artifact body into the import stage, then answers once.
///
/// Nothing is read before the caller is admitted, so a refused caller's body is never written
/// anywhere; the connection closes after the answer.
pub fn serve_receive(
    reader: &mut BufReader<&TcpStream>,
    request: &Request,
    import: &soma::OperationId,
    artifact: TransferArtifact,
) {
    let operation = TransferRoute::Receive(import.clone(), artifact).operation();
    let stream: &TcpStream = reader.get_ref();
    let outcome = platform_only(request)
        .and_then(|()| announced_length(request))
        .and_then(|length| receive(reader, import, artifact, length));
    answer(stream, operation, outcome);
}

fn announced_length(request: &Request) -> Result<u64, ApiError> {
    if request.header("transfer-encoding").is_some() {
        return Err(ApiError::new(
            411,
            "length_required",
            "an artifact must be sent with a content-length",
            false,
        ));
    }
    let length: u64 = request
        .header("content-length")
        .ok_or(ApiError::new(
            411,
            "length_required",
            "an artifact must be sent with a content-length",
            false,
        ))?
        .parse()
        .map_err(|_| ApiError::invalid("the content-length header is not a length"))?;
    if length > MAX_ARTIFACT_BYTES {
        return Err(ApiError::new(
            413,
            "body_limit",
            "the artifact exceeded the largest artifact a host accepts",
            false,
        ));
    }
    Ok(length)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn receive(
    reader: &mut BufReader<&TcpStream>,
    import: &soma::OperationId,
    artifact: TransferArtifact,
    length: u64,
) -> Result<Response, ApiError> {
    let stage = soma_local::ImportStage::begin(import).map_err(import_refusal)?;
    let digest = stage
        .receive(local_artifact(artifact), length, &mut reader.take(length))
        .map_err(import_refusal)?;
    let received = Received {
        import_id: import.as_str(),
        artifact: artifact.name(),
        bytes: length,
        digest,
    };
    let operation = TransferRoute::Receive(import.clone(), artifact).operation();
    Ok(Response::new(
        200,
        render(&Envelope::success(operation, &received, None)),
    ))
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn receive(
    _reader: &mut BufReader<&TcpStream>,
    _import: &soma::OperationId,
    _artifact: TransferArtifact,
    _length: u64,
) -> Result<Response, ApiError> {
    Err(refusal(soma_local::CheckpointFailure::Unsupported))
}

#[cfg(test)]
#[path = "transfer/tests.rs"]
mod tests;
