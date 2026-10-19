//! The graceful stop: one [`Shutdown`] handle per server, asked by `POST /__kaanal/shutdown`
//! or [`crate::BoundServer::shutdown`].
//!
//! The first request fixes a deadline (`now + budget`); later ones never move it. Until the
//! deadline the server drains: no new connections, in-flight requests finish. At the deadline
//! every connection still open is cut ([`Cut`]), which hyper treats like a client disconnect
//! (the handler future is dropped, so the request log's drop guard records `client-gone`).

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::serve::Listener;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::time::Instant;

/// A shutdown request and its deadline. Cloning shares the same state.
#[derive(Clone)]
pub struct Shutdown {
    /// `None` until someone asks; then the fixed deadline.
    tx: Arc<watch::Sender<Option<Instant>>>,
}

impl Shutdown {
    /// No shutdown requested yet.
    pub fn new() -> Shutdown {
        Shutdown {
            tx: Arc::new(watch::channel(None).0),
        }
    }

    /// Asks the server to stop within `budget`. The first call fixes the deadline and returns
    /// `budget`; later calls return the time left and never move the deadline.
    pub fn request(&self, budget: Duration) -> Duration {
        let now = Instant::now();
        let mut first = false;
        self.tx.send_if_modified(|d| {
            first = d.is_none();
            if first {
                *d = Some(now + budget);
            }
            first
        });
        if first {
            budget
        } else {
            self.remaining().unwrap_or_default()
        }
    }

    /// Time left to the deadline (zero once past); `None` before any request.
    pub fn remaining(&self) -> Option<Duration> {
        (*self.tx.borrow()).map(|d| d.saturating_duration_since(Instant::now()))
    }

    /// A receiver that sees the deadline appear (for streams that must end on a request).
    pub fn watch(&self) -> watch::Receiver<Option<Instant>> {
        self.tx.subscribe()
    }

    /// Resolves once a shutdown has been requested.
    pub async fn requested(&self) {
        self.deadline().await;
    }

    /// Resolves when the deadline passes (never, if nothing is requested).
    pub async fn expired(&self) {
        tokio::time::sleep_until(self.deadline().await).await;
    }

    /// **Private.** Waits for a request and returns its deadline.
    async fn deadline(&self) -> Instant {
        let mut rx = self.tx.subscribe();
        // The sender lives in `self`, so the channel cannot close under us.
        let seen = rx.wait_for(Option::is_some).await.ok().and_then(|d| *d);
        seen.expect("the shutdown channel closed")
    }
}

/// **Private.** A listener whose connections stop working once the deadline passes. axum
/// spawns each connection on its own task, so aborting the serve task would leave them
/// running; failing their I/O is what makes hyper drop the in-flight handlers.
pub(crate) struct Cut {
    inner: TcpListener,
    sd: Shutdown,
}

impl Cut {
    pub(crate) fn new(inner: TcpListener, sd: Shutdown) -> Cut {
        Cut { inner, sd }
    }
}

impl Listener for Cut {
    type Io = CutIo;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (CutIo, SocketAddr) {
        let (io, addr) = Listener::accept(&mut self.inner).await;
        let sd = self.sd.clone();
        let io = CutIo {
            io,
            expired: Box::pin(async move { sd.expired().await }),
            cut: false,
        };
        (io, addr)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Listener::local_addr(&self.inner)
    }
}

/// **Private.** One accepted connection; reads and writes fail with `ConnectionAborted` once
/// the shutdown deadline has passed.
pub(crate) struct CutIo {
    io: TcpStream,
    expired: Pin<Box<dyn Future<Output = ()> + Send>>,
    cut: bool,
}

impl CutIo {
    /// Polling registers the waker, so the connection wakes at the deadline even when idle.
    fn gone(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        self.cut = self.cut || self.expired.as_mut().poll(cx).is_ready();
        if self.cut {
            Err(io::ErrorKind::ConnectionAborted.into())
        } else {
            Ok(())
        }
    }
}

impl AsyncRead for CutIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(e) = self.gone(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for CutIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(e) = self.gone(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(e) = self.gone(cx) {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::Shutdown;
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn first_request_fixes_the_deadline_and_later_ones_do_not_extend_it() {
        let sd = Shutdown::new();
        assert_eq!(sd.request(Duration::from_secs(5)), Duration::from_secs(5));
        tokio::time::advance(Duration::from_secs(2)).await;
        // A longer budget later neither extends nor reports itself: the time left is returned.
        assert_eq!(sd.request(Duration::from_secs(60)), Duration::from_secs(3));
        assert_eq!(sd.remaining(), Some(Duration::from_secs(3)));
        tokio::time::advance(Duration::from_secs(10)).await;
        assert_eq!(sd.request(Duration::from_secs(1)), Duration::ZERO);
        assert_eq!(sd.remaining(), Some(Duration::ZERO));
    }

    #[test]
    fn remaining_is_none_before_a_request() {
        assert_eq!(Shutdown::new().remaining(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn requested_resolves_after_request() {
        let sd = Shutdown::new();
        let waiter = tokio::spawn({
            let sd = sd.clone();
            async move { sd.requested().await }
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        sd.request(Duration::from_secs(1));
        waiter.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn expired_resolves_at_the_deadline() {
        let sd = Shutdown::new();
        sd.request(Duration::from_secs(5));
        let t0 = tokio::time::Instant::now();
        sd.expired().await;
        assert_eq!(t0.elapsed(), Duration::from_secs(5));
    }
}
