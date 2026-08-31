use std::{
    io::{BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::Duration,
};

use crate::{
    envelope::{ApiError, failure_body},
    facade::SandboxFacade,
    handler::handle,
    http::{request::Request, response::Response},
};

/// How long one connection may take to send its request or accept its response.
///
/// A client that opens a socket and then stalls must not hold a thread and, more importantly,
/// must not hold an open runtime against the durable state store.
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Accepts connections until the listener fails, serving each on its own thread.
///
/// A thread per connection is the right shape here: every operation this service performs is a
/// blocking call into the local runtime, so an asynchronous runtime would spend its time on
/// blocking-pool handoffs and buy nothing. The connection count is bounded in practice by the
/// number of sandboxes a host can run at all.
///
/// # Errors
///
/// Returns the listener failure that ended the loop.
pub fn serve<M, F>(listener: &TcpListener, open_facade: M) -> std::io::Result<()>
where
    M: Fn() -> Result<F, ApiError> + Send + Sync + 'static,
    F: SandboxFacade + 'static,
{
    let open_facade = Arc::new(open_facade);
    for stream in listener.incoming() {
        let stream = stream?;
        let open_facade = Arc::clone(&open_facade);
        // The join handle is dropped on purpose. A connection that outlives the accept loop has
        // nothing left to report to it, and joining here would serialize the whole service.
        drop(thread::spawn(move || {
            serve_connection(&stream, open_facade.as_ref());
        }));
    }
    Ok(())
}

/// Serves exactly one request on one connection.
///
/// The facade is opened only after the request parses. Opening it first would mean a malformed
/// request could still cost a state-store handle.
fn serve_connection<M, F>(stream: &TcpStream, open_facade: &M)
where
    M: Fn() -> Result<F, ApiError>,
    F: SandboxFacade,
{
    if stream.set_read_timeout(Some(CONNECTION_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(CONNECTION_TIMEOUT)).is_err()
    {
        return;
    }
    let mut reader = BufReader::new(stream);
    let response = match Request::read_from(&mut reader) {
        Ok(request) => match open_facade() {
            Ok(mut facade) => handle(&mut facade, &request),
            Err(error) => Response::new(error.status(), failure_body("request", error)),
        },
        Err(error) => Response::new(error.status(), failure_body("request", error)),
    };
    let mut writer = stream;
    // A write failure means the peer is gone. There is nobody left to tell, so the connection is
    // simply dropped rather than logged as a service fault.
    if response.write_to(&mut writer).is_ok() {
        let _ = writer.flush();
    }
}
