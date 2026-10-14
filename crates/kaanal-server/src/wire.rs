//! A response body that breaks off halfway, for the `reset` failure kind.
//!
//! The client must see the status line and part of the body before the connection aborts.
//! The plan-task-0 spike found that hyper 1.x only flushes the head when the stream is shaped as
//! one chunk, then one `Pending` (waker woken), then the error: an error on the first poll, or
//! straight after the chunk, drops the head and the client sees an empty reply instead.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::body::Bytes;
use tokio_stream::Stream;

/// One chunk, one `Pending`, then a `ConnectionReset` error. With a `Content-Length` larger than
/// the chunk, the client reads part of the body and then sees the transfer abort. This is not a
/// literal TCP RST (R1.18).
pub(crate) struct Cut {
    chunk: Option<Bytes>,
    parked: bool,
}

impl Cut {
    /// A stream that delivers `chunk` and then breaks.
    pub(crate) fn new(chunk: Bytes) -> Cut {
        Cut {
            chunk: Some(chunk),
            parked: false,
        }
    }
}

impl Stream for Cut {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(c) = self.chunk.take() {
            return Poll::Ready(Some(Ok(c)));
        }
        if !self.parked {
            self.parked = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Some(Err(io::Error::new(
            io::ErrorKind::ConnectionReset,
            "kaanal chaos: reset",
        ))))
    }
}
