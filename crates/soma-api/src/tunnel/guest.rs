//! The guest end of a tunnel: a Unix socket to the process that hosts the machine.
//!
//! Off unix there is no such socket, so no tunnel can open. There the stream type cannot be
//! constructed: a facade can only refuse, and the pumps that would move its bytes are proven
//! unreachable by the type rather than by a check at run time.

/// A dialled guest stream, as a facade hands it back.
#[cfg(unix)]
pub type GuestStream = std::os::unix::net::UnixStream;

/// A dialled guest stream registered with the public runner's asynchronous runtime.
#[cfg(unix)]
pub type AsyncGuest = tokio::net::UnixStream;

/// Registers a dialled guest stream with the asynchronous runtime the public runner pumps on.
///
/// # Errors
///
/// Returns the I/O failure of making the stream nonblocking or of registering it.
#[cfg(unix)]
pub fn into_async(stream: GuestStream) -> std::io::Result<AsyncGuest> {
    stream.set_nonblocking(true)?;
    tokio::net::UnixStream::from_std(stream)
}

/// A guest stream, which off unix no facade can produce.
#[cfg(not(unix))]
#[derive(Debug)]
pub struct GuestStream {
    never: std::convert::Infallible,
}

/// The asynchronous guest stream, which off unix is the same unconstructible stream.
#[cfg(not(unix))]
pub type AsyncGuest = GuestStream;

/// Never runs off unix, because no guest stream exists to register.
///
/// # Errors
///
/// None: no stream can be passed in.
#[cfg(not(unix))]
pub fn into_async(GuestStream { never }: GuestStream) -> std::io::Result<AsyncGuest> {
    match never {}
}

#[cfg(not(unix))]
impl std::io::Write for &GuestStream {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        match self.never {}
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.never {}
    }
}

/// Never runs off unix, because no guest stream exists to pump.
#[cfg(not(unix))]
pub fn pump_blocking(
    _client: &std::net::TcpStream,
    guest: &GuestStream,
    _idle: std::time::Duration,
) {
    match guest.never {}
}

/// Never runs off unix, because no guest stream exists to pump.
#[cfg(not(unix))]
pub fn pump<C>(
    _client: C,
    GuestStream { never }: AsyncGuest,
    _idle: std::time::Duration,
    _touch: impl Fn(),
) -> std::future::Ready<()> {
    match never {}
}
