//! Reading a request body with a hard size cap, for the routes the server answers itself (`POST
//! /login`, an item added to a collection). The cap holds whatever `Content-Length` says, and a body
//! that is too large is told apart from one whose stream broke (a client that hung up), so the first
//! is a 413 and the second a 400.

use axum::extract::Request;
use tokio_stream::StreamExt;

/// Why a body could not be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    /// More than the cap.
    TooLarge,
    /// The stream failed before the end (the client went away, or sent a broken frame).
    Broken,
}

/// The whole body, or why not. Stops reading at the first byte over `max`.
pub(crate) async fn read_capped(req: Request, max: usize) -> Result<Vec<u8>, ReadError> {
    let mut stream = req.into_body().into_data_stream();
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ReadError::Broken)?;
        if out.len() + chunk.len() > max {
            return Err(ReadError::TooLarge);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, Bytes};

    fn req(parts: Vec<Result<&'static str, &'static str>>) -> Request {
        let chunks = parts.into_iter().map(|p| {
            p.map(Bytes::from)
                .map_err(|e| std::io::Error::other(e.to_string()))
        });
        Request::new(Body::from_stream(tokio_stream::iter(chunks)))
    }

    #[tokio::test]
    async fn a_body_at_the_cap_is_read_in_full_across_chunks() {
        let r = read_capped(req(vec![Ok("ab"), Ok("cd")]), 4).await;
        assert_eq!(r, Ok(b"abcd".to_vec()));
    }

    #[tokio::test]
    async fn one_byte_over_the_cap_is_too_large() {
        let r = read_capped(req(vec![Ok("ab"), Ok("cde")]), 4).await;
        assert_eq!(r, Err(ReadError::TooLarge));
    }

    #[tokio::test]
    async fn a_stream_that_breaks_is_not_called_too_large() {
        let r = read_capped(req(vec![Ok("ab"), Err("reset")]), 4).await;
        assert_eq!(r, Err(ReadError::Broken));
    }

    #[tokio::test]
    async fn an_empty_body_is_empty() {
        assert_eq!(read_capped(req(vec![]), 4).await, Ok(Vec::new()));
    }
}
