//! The live routes: `GET /__kaanal/live` (server-sent events) and the commands `pause`, `resume`
//! and `rate`. The stream holds a [`super::ClientGuard`] for as long as it is open, so the session
//! lives exactly while a page is connected (plus the grace period). The token may be given as
//! `?token=` on the stream only, as on `/events`, because `EventSource` cannot set headers.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use serde::Deserialize;
use serde_json::Value;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::{BroadcastStream, WatchStream};
use tokio_stream::{Stream, StreamExt};

use super::{LiveError, LiveEvent};
use crate::control::reject;
use crate::state::AppState;

/// The query of the stream: `fresh` (any value) marks a page load, which resets the run.
#[derive(Deserialize)]
pub struct LiveQuery {
    fresh: Option<String>,
}

/// `GET /__kaanal/live`: a `hello` event with the session, then (from L3) `live` ticks; a
/// keep-alive comment every 2 s. Ends when the server shuts down.
pub async fn live(
    State(st): State<Arc<AppState>>,
    Query(q): Query<LiveQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let guard = st.live.attach(&st, q.fresh.is_some());
    let hello = SseEvent::default()
        .event("hello")
        .json_data(st.live.status().unwrap_or(Value::Null))
        .unwrap_or_default();
    let stop = WatchStream::new(st.shutdown.watch())
        .filter(Option::is_some)
        .map(|_| None);
    // no session (cannot happen right after attach): a receiver whose sender is gone, so the
    // stream ends at once and the page reconnects
    let rx = st
        .live
        .subscribe()
        .unwrap_or_else(|| tokio::sync::broadcast::channel(1).1);
    let follow = BroadcastStream::new(rx).map({
        let st = Arc::clone(&st);
        move |r| Some(Ok(to_sse(&r, &st)))
    });
    // the stream ends with its session (a restart after an Apply, the last client leaving), so
    // the browser reconnects to the new one; `None` is what ends it
    let opened = tokio_stream::iter([Some(Ok(hello))])
        .chain(follow)
        .chain(tokio_stream::iter([None]));
    let stream = opened.merge(stop).map_while(|item| item).map(move |item| {
        let _alive = &guard; // dropped with the stream: the client has left
        item
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(2)))
}

/// **Private.** The name of the event a received broadcast item becomes: `live` for a tick,
/// `marker`, and `resync` after the client fell behind and missed some.
pub(super) fn event_name(r: &Result<Arc<LiveEvent>, BroadcastStreamRecvError>) -> &'static str {
    match r {
        Ok(ev) => match &**ev {
            LiveEvent::Tick(_) => "live",
            LiveEvent::Marker(_) => "marker",
        },
        Err(BroadcastStreamRecvError::Lagged(_)) => "resync",
    }
}

fn to_sse(r: &Result<Arc<LiveEvent>, BroadcastStreamRecvError>, st: &AppState) -> SseEvent {
    let e = SseEvent::default().event(event_name(r));
    match r {
        Ok(ev) => match &**ev {
            LiveEvent::Tick(t) => e.json_data(t),
            LiveEvent::Marker(m) => e.json_data(m),
        },
        Err(_) => e.json_data(st.live.status().unwrap_or(Value::Null)),
    }
    .unwrap_or_default()
}

fn answer(r: Result<(), LiveError>, st: &AppState) -> axum::response::Response {
    match r {
        Ok(()) => Json(st.live.status().unwrap_or(Value::Null)).into_response(),
        Err(LiveError::NoSession) => reject(
            StatusCode::CONFLICT,
            "no live session: no page is connected to /__kaanal/live",
        )
        .into_response(),
        Err(LiveError::Refused(m)) => reject(StatusCode::BAD_REQUEST, &m).into_response(),
    }
}

/// `POST /__kaanal/live/pause`.
pub async fn pause(State(st): State<Arc<AppState>>) -> axum::response::Response {
    answer(st.live.pause(), &st)
}

/// `POST /__kaanal/live/resume`.
pub async fn resume(State(st): State<Arc<AppState>>) -> axum::response::Response {
    answer(st.live.resume(), &st)
}

/// The body of `PUT /__kaanal/live/rate`.
#[derive(Deserialize)]
pub struct RateBody {
    rps: f64,
}

/// `PUT /__kaanal/live/rate` with `{ "rps": N }`: the offered rate, at most 20,000 a second.
pub async fn rate(
    State(st): State<Arc<AppState>>,
    Json(b): Json<RateBody>,
) -> axum::response::Response {
    answer(st.live.set_rate(b.rps), &st)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::tick::{Tick, TickStatus};

    #[test]
    fn a_tick_is_live_a_mark_is_marker_and_a_lag_is_resync() {
        let tick = Tick {
            t_ms: 250,
            system_ms: 250,
            requests: 0,
            failed: 0,
            nodes: Vec::new(),
            wires: Vec::new(),
            hist: Vec::new(),
            journeys: Vec::new(),
            status: TickStatus {
                rate: 1.0,
                paused: false,
                overloaded: false,
                in_flight: 0,
            },
        };
        let t = Ok(Arc::new(LiveEvent::Tick(Box::new(tick))));
        assert_eq!(event_name(&t), "live");
        let m = Ok(Arc::new(LiveEvent::Marker(crate::live::Marker {
            at_ms: 1,
            kind: "pause",
            text: "p".into(),
        })));
        assert_eq!(event_name(&m), "marker");
        assert_eq!(
            event_name(&Err(BroadcastStreamRecvError::Lagged(7))),
            "resync"
        );
    }
}
