//! The public listener's bounds (contract C2): connections per IP, header and body sizes,
//! read and idle timeouts, and streams per connection.

use std::{
    collections::HashMap,
    io,
    net::IpAddr,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A connection with no bytes moving either way for this long is closed once its in-flight
/// requests finish; the loopback service uses the same 60 s.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How often an idle connection is looked at.
pub const IDLE_CHECK: Duration = Duration::from_secs(5);
pub const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// An HTTP/1.1 request's headers must arrive within this.
pub const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// A request body must arrive within this once its headers have.
pub const BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest HTTP/1.1 read buffer, which bounds the request line and headers.
pub const MAX_HTTP1_BUFFER_BYTES: usize = 64 * 1024;
/// The largest decoded header block on HTTP/2 and HTTP/3.
pub const MAX_HEADER_BYTES: u32 = 16 * 1024;
/// Concurrent requests on one HTTP/2 connection or HTTP/3 connection.
pub const MAX_STREAMS_PER_CONNECTION: u32 = 128;
/// The default for connections one client IP may hold, TCP and QUIC together.
pub const DEFAULT_CONNECTIONS_PER_IP: usize = 256;

/// Counts open connections per client IP and refuses past the limit.
#[derive(Debug)]
pub struct PerIp {
    limit: usize,
    open: Mutex<HashMap<IpAddr, usize>>,
}

/// One admitted connection; dropping it gives the slot back.
#[derive(Debug)]
pub struct IpSlot {
    owner: Arc<PerIp>,
    ip: IpAddr,
}

impl PerIp {
    #[must_use]
    pub fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            open: Mutex::new(HashMap::new()),
        })
    }

    /// Admits one more connection from `ip`, or `None` when it already holds the limit.
    #[must_use]
    pub fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<IpSlot> {
        let mut open = self.lock();
        let count = open.entry(ip).or_insert(0);
        if *count >= self.limit {
            return None;
        }
        *count += 1;
        Some(IpSlot {
            owner: Arc::clone(self),
            ip,
        })
    }

    #[must_use]
    pub fn open_from(&self, ip: IpAddr) -> usize {
        self.lock().get(&ip).copied().unwrap_or(0)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, usize>> {
        self.open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for IpSlot {
    fn drop(&mut self) {
        let mut open = self.owner.lock();
        if let Some(count) = open.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                open.remove(&self.ip);
            }
        }
    }
}

/// When a connection last moved a byte.
#[derive(Debug)]
pub struct Activity {
    started: Instant,
    last_millis: AtomicU64,
}

impl Activity {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            last_millis: AtomicU64::new(0),
        })
    }

    fn stamp(&self) {
        let millis = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_millis.store(millis, Ordering::Relaxed);
    }

    /// How long no byte has moved.
    #[must_use]
    pub fn idle_for(&self) -> Duration {
        let last = Duration::from_millis(self.last_millis.load(Ordering::Relaxed));
        self.started.elapsed().saturating_sub(last)
    }
}

/// A stream that records its activity, so an idle connection can be found and closed.
pub struct IdleIo<T> {
    inner: T,
    activity: Arc<Activity>,
}

impl<T> IdleIo<T> {
    pub fn new(inner: T, activity: &Arc<Activity>) -> Self {
        activity.stamp();
        Self {
            inner,
            activity: Arc::clone(activity),
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for IdleIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buffer.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(context, buffer);
        if buffer.filled().len() > before {
            self.activity.stamp();
        }
        polled
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for IdleIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.inner).poll_write(context, bytes);
        if matches!(polled, Poll::Ready(Ok(written)) if written > 0) {
            self.activity.stamp();
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use super::PerIp;

    #[test]
    fn an_ip_holds_at_most_its_limit_and_slots_come_back() {
        let limiter = PerIp::new(2);
        let client = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        let other = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 8));

        let first = limiter.admit(client).expect("first");
        let _second = limiter.admit(client).expect("second");
        assert!(limiter.admit(client).is_none());
        assert!(limiter.admit(other).is_some(), "IPs are counted apart");
        drop(first);
        assert_eq!(limiter.open_from(client), 1);
        assert!(limiter.admit(client).is_some());
    }
}
