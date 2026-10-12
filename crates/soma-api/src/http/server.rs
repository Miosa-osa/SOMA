use std::{
    io::{BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::Duration,
};

use crate::{
    admission::{CreateAdmission, CreatePermit},
    envelope::{ApiError, failure_body},
    facade::SandboxFacade,
    handler::{handle, handle_prewarm},
    http::{request::Request, response::Response},
    route::{Route, resolve},
    tenant::{TENANT_HEADER, identify},
    tunnel::{self, Refusal, TunnelPolicy, TunnelTable},
};

/// How long one connection may take to send its request or accept its response.
///
/// A client that opens a socket and then stalls must not hold a thread and, more importantly,
/// must not hold an open runtime against the durable state store.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);
/// One connection cannot monopolize a worker forever, even when every request is valid.
const MAX_REQUESTS_PER_CONNECTION: usize = 16;

/// Accepts connections until the listener fails, serving each on its own thread.
///
/// Creates are bounded by `admission`; every other route is bounded only by the facade pool.
///
/// A thread per connection is the right shape here: every operation this service performs is a
/// blocking call into the local runtime, so an asynchronous runtime would spend its time on
/// blocking-pool handoffs and buy nothing. The connection count is bounded in practice by the
/// number of sandboxes a host can run at all.
///
/// # Errors
///
/// Returns the listener failure that ended the loop.
pub fn serve<M, F>(
    listener: &TcpListener,
    open_facade: M,
    admission: &CreateAdmission,
) -> std::io::Result<()>
where
    M: Fn() -> Result<F, ApiError> + Send + Sync + 'static,
    F: SandboxFacade + 'static,
{
    serve_with_tunnels(
        listener,
        open_facade,
        admission,
        &TunnelTable::new(TunnelPolicy::default()),
    )
}

/// [`serve`], with the policy and limits guest-port tunnels are held to.
///
/// # Errors
///
/// Returns the listener failure that ended the loop.
pub fn serve_with_tunnels<M, F>(
    listener: &TcpListener,
    open_facade: M,
    admission: &CreateAdmission,
    tunnels: &Arc<TunnelTable>,
) -> std::io::Result<()>
where
    M: Fn() -> Result<F, ApiError> + Send + Sync + 'static,
    F: SandboxFacade + 'static,
{
    let open_facade = Arc::new(open_facade);
    for stream in listener.incoming() {
        let stream = stream?;
        let open_facade = Arc::clone(&open_facade);
        let admission = admission.clone();
        let tunnels = Arc::clone(tunnels);
        // The join handle is dropped on purpose. A connection that outlives the accept loop has
        // nothing left to report to it, and joining here would serialize the whole service.
        drop(thread::spawn(move || {
            serve_connection(&stream, open_facade.as_ref(), &admission, &tunnels);
        }));
    }
    Ok(())
}

/// Serves a bounded sequence of framed requests on one connection.
///
/// The facade is opened only after the request parses. Opening it first would mean a malformed
/// request could still cost a state-store handle.
fn serve_connection<M, F>(
    stream: &TcpStream,
    open_facade: &M,
    admission: &CreateAdmission,
    tunnels: &Arc<TunnelTable>,
) where
    M: Fn() -> Result<F, ApiError>,
    F: SandboxFacade,
{
    if stream.set_read_timeout(Some(CONNECTION_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(CONNECTION_TIMEOUT)).is_err()
        || stream.set_nodelay(true).is_err()
    {
        return;
    }
    let mut reader = BufReader::new(stream);
    for request_index in 0..MAX_REQUESTS_PER_CONNECTION {
        let request = match Request::read_from(&mut reader) {
            Ok(request) => request,
            Err(error) => {
                let response = Response::new(error.status(), failure_body("request", error));
                let _ignored = response.write_to(&mut &*stream);
                return;
            }
        };
        if let Ok(Route::Tunnel(instance_id, port)) = resolve(&request) {
            // A tunnel takes the connection over: nothing else is read from it as HTTP.
            serve_tunnel(
                &reader,
                &request,
                open_facade,
                tunnels,
                (&instance_id, port),
            );
            return;
        }
        let keep_alive = request.keep_alive() && request_index + 1 < MAX_REQUESTS_PER_CONNECTION;
        let response = if matches!(resolve(&request), Ok(Route::Prewarm(_))) {
            // Needs no facade: see the route's documentation.
            handle_prewarm(&request)
        } else {
            match admit(&request, admission) {
                Ok(_permit) => match open_facade() {
                    Ok(mut facade) => handle(&mut facade, &request),
                    Err(error) => Response::new(error.status(), failure_body("request", error)),
                },
                Err(busy) => busy,
            }
        };
        let written = if keep_alive {
            response.write_keep_alive_to(&mut &*stream)
        } else {
            response.write_to(&mut &*stream)
        };
        if written.is_err() || !keep_alive {
            return;
        }
    }
}

/// Holds a create's place for as long as the create is served, or refuses it at once.
///
/// Only an identified create is counted. Anything else, including a create the handler is about
/// to refuse as unidentified, passes with no permit, so a caller learns it is unidentified before
/// it learns anything about this runner's load.
fn admit(request: &Request, admission: &CreateAdmission) -> Result<Option<CreatePermit>, Response> {
    let is_create = identify(request.header(TENANT_HEADER)).is_ok()
        && matches!(resolve(request), Ok(Route::CreateSandbox));
    if !is_create {
        return Ok(None);
    }
    admission.try_admit().map(Some).ok_or_else(|| {
        let busy = CreateAdmission::busy();
        // Zero, because the refusal says nothing about when this runner frees up: the caller
        // should go to another runner now rather than wait for this one.
        Response::new(busy.status(), failure_body("sandbox.create", busy)).with_retry_after(0)
    })
}

/// Answers a tunnel request: refuses it, or switches the connection to the guest's stream.
fn serve_tunnel<M, F>(
    reader: &BufReader<&TcpStream>,
    request: &Request,
    open_facade: &M,
    tunnels: &Arc<TunnelTable>,
    (instance_id, port): (&soma::InstanceId, u16),
) where
    M: Fn() -> Result<F, ApiError>,
    F: SandboxFacade,
{
    let stream: &TcpStream = reader.get_ref();
    let refuse = |error: ApiError| {
        let response = Response::new(error.status(), failure_body("sandbox.tunnel", error));
        let _ignored = response.write_to(&mut &*stream);
    };
    if let Err(error) = identify(request.header(TENANT_HEADER)) {
        return refuse(error);
    }
    if !tunnel::asks_for_upgrade(request.header("connection"), request.header("upgrade")) {
        return refuse(ApiError::new(
            426,
            "upgrade_required",
            "a tunnel needs `Connection: Upgrade` and `Upgrade: soma-tunnel`",
            false,
        ));
    }
    let slot = match tunnels.admit(instance_id.as_str(), port) {
        Ok(slot) => slot,
        Err(refused) => return refuse(Refusal::from(refused).api_error()),
    };
    // The guest is dialled before the 101, so a refusal reaches the caller as an HTTP status.
    let guest = match open_facade() {
        Ok(mut facade) => facade.tunnel(instance_id, port),
        Err(error) => return refuse(error),
    };
    let guest = match guest {
        Ok(guest) => guest,
        Err(failure) => return refuse(Refusal::from(failure).api_error()),
    };
    let answer = format!(
        "HTTP/1.1 101 Switching Protocols\r\nconnection: Upgrade\r\nupgrade: {}\r\n\r\n",
        tunnel::UPGRADE_TOKEN
    );
    let mut writer = stream;
    if writer.write_all(answer.as_bytes()).is_err() {
        return;
    }
    // Bytes the caller sent behind its request are the stream's first bytes.
    let early = reader.buffer();
    if !early.is_empty() && (&guest).write_all(early).is_err() {
        return;
    }
    // A tunnel idles for as long as its user does; the connection timeouts no longer apply.
    if stream.set_write_timeout(None).is_err() {
        return;
    }
    tunnel::pump_blocking(stream, &guest, tunnels.policy().idle);
    drop(slot);
}
