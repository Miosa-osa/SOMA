//! The byte pumps behind a tunnel once its `101` is sent.
//!
//! Unix only: the guest end is a Unix socket to the process that hosts the machine.

use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CHUNK: usize = 16 * 1024;
/// How often a blocking pump wakes to notice an idle tunnel.
const POLL: Duration = Duration::from_secs(1);
/// How often traffic is reported as sandbox activity.
const TOUCH_EVERY: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------------------------
// Blocking pump (the loopback face)
// ---------------------------------------------------------------------------------------------

/// Pumps bytes between `client` and `guest` until both directions end or `idle` passes with no
/// bytes moving in either.
pub fn pump_blocking(client: &TcpStream, guest: &UnixStream, idle: Duration) {
    if client.set_read_timeout(Some(POLL)).is_err() || guest.set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let (Ok(mut client_read), Ok(mut guest_write)) = (client.try_clone(), guest.try_clone()) else {
        return;
    };
    let (Ok(mut guest_read), Ok(mut client_write)) = (guest.try_clone(), client.try_clone()) else {
        return;
    };
    let clock = Arc::new(Clock::new());
    let stop = Arc::new(AtomicBool::new(false));
    let upstream = {
        let (clock, stop) = (Arc::clone(&clock), Arc::clone(&stop));
        thread::spawn(move || {
            copy_blocking(&mut client_read, &mut guest_write, &clock, &stop, idle);
            let _ignored = guest_write.shutdown(Shutdown::Write);
        })
    };
    copy_blocking(&mut guest_read, &mut client_write, &clock, &stop, idle);
    // The guest ended its side, so nothing the client sends can be answered: the tunnel is over,
    // and the other direction notices at its next poll. A client that ends ITS side first keeps
    // receiving until the guest is done.
    stop.store(true, Ordering::Release);
    let _ignored = client_write.shutdown(Shutdown::Write);
    let _joined = upstream.join();
    let _ignored = client.shutdown(Shutdown::Both);
    let _ignored = guest.shutdown(Shutdown::Both);
}

fn copy_blocking(
    from: &mut impl Read,
    to: &mut impl Write,
    clock: &Clock,
    stop: &AtomicBool,
    idle: Duration,
) {
    let mut buffer = vec![0_u8; CHUNK];
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        match from.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => {
                if to.write_all(&buffer[..read]).is_err() {
                    stop.store(true, Ordering::Release);
                    return;
                }
                clock.mark();
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                if clock.idle_for() >= idle {
                    stop.store(true, Ordering::Release);
                    return;
                }
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                return;
            }
        }
    }
}

/// Milliseconds since the pump started at which bytes last moved.
struct Clock {
    started: Instant,
    last: AtomicU64,
}

impl Clock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            last: AtomicU64::new(0),
        }
    }

    fn mark(&self) {
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last.store(now, Ordering::Release);
    }

    fn idle_for(&self) -> Duration {
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Duration::from_millis(now.saturating_sub(self.last.load(Ordering::Acquire)))
    }
}

// ---------------------------------------------------------------------------------------------
// Asynchronous pump (the public runner)
// ---------------------------------------------------------------------------------------------

/// Pumps bytes between `client` and `guest` until both directions end, one errors, or `idle`
/// passes with no bytes moving. `touch` is called while bytes move, at most every
/// `TOUCH_EVERY` (15 s), so a busy tunnel keeps its sandbox from idling out.
pub async fn pump<C>(client: C, guest: tokio::net::UnixStream, idle: Duration, touch: impl Fn())
where
    C: AsyncRead + AsyncWrite + Unpin,
{
    let clock = Clock::new();
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut guest_read, mut guest_write) = guest.into_split();
    {
        let upstream = copy_async(&mut client_read, &mut guest_write, &clock, &touch);
        let downstream = copy_async(&mut guest_read, &mut client_write, &clock, &touch);
        let watchdog = async {
            loop {
                tokio::time::sleep(POLL).await;
                if clock.idle_for() >= idle {
                    return;
                }
            }
        };
        tokio::pin!(upstream, downstream, watchdog);
        // The guest ending its side ends the tunnel (nothing the client sends could be
        // answered); the client ending its side only stops the upstream, and the guest may
        // still answer.
        let mut upstream_open = true;
        loop {
            tokio::select! {
                () = &mut downstream => break,
                () = &mut upstream, if upstream_open => upstream_open = false,
                () = &mut watchdog => break,
            }
        }
    }
    // Both ends are closed in order, so the client sees a clean end (a TLS `close_notify`
    // among them) rather than a connection that vanished.
    let _ignored = client_write.shutdown().await;
    let _ignored = guest_write.shutdown().await;
}

async fn copy_async<R, W>(from: &mut R, to: &mut W, clock: &Clock, touch: &impl Fn())
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = vec![0_u8; CHUNK];
    let mut touched = Instant::now();
    loop {
        match from.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if to.write_all(&buffer[..read]).await.is_err() {
                    return;
                }
                clock.mark();
                if touched.elapsed() >= TOUCH_EVERY {
                    touch();
                    touched = Instant::now();
                }
            }
        }
    }
    let _ignored = to.shutdown().await;
}

#[cfg(test)]
#[path = "pump_tests.rs"]
mod tests;
