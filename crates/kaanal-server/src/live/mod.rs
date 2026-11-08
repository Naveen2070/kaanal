//! The live simulation (step 8L, RULES 105): a generator feeding the shared engine, the ticks
//! that count what it does, and the stream that carries them to Studio.
//!
//! One **session** at most: a generator over the first traffic profile of the current system. It
//! starts when the first stream client connects, a client that says `fresh` (a page load) resets
//! it, and it stops [`GRACE`] after the last client leaves, so a closed page never leaves a
//! generator running (the mocks and the engine keep serving). An automatic reconnect of the same
//! page (no `fresh`) keeps the session and its timeline.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;

use crate::generation::Generation;
use crate::sim::profile::{Profile, RunRequest, to_sim};
use crate::state::{AppState, BusEvent};

pub(crate) mod edit;
pub(crate) mod generator;
pub(crate) mod hist;
pub(crate) mod stream;
pub(crate) mod tick;

use generator::{GeneratorHandle, Limits, RateRefused, StreamShare};
use tick::{Aggregator, Tick, TickStatus};

/// The length of a tick.
const TICK: Duration = Duration::from_millis(250);

/// A change worth a mark on the timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Marker {
    /// Milliseconds since the session started.
    pub at_ms: u64,
    /// `param`, `chaos`, `structure`, `pause` or `resume`.
    pub kind: &'static str,
    pub text: String,
}

/// What a session broadcasts to the pages that follow it.
#[derive(Debug, Clone)]
pub(crate) enum LiveEvent {
    Tick(Box<Tick>),
    Marker(Marker),
}

/// How long a session outlives its last client.
pub(crate) const GRACE: Duration = Duration::from_secs(5);

/// Why a live command could not be done.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LiveError {
    /// No session is running (no page is connected).
    NoSession,
    /// The rate was refused, with the reason.
    Refused(String),
}

/// The one live session of the server, if any.
#[derive(Default)]
pub struct LiveSlot {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    session: Option<Session>,
    /// Stream clients connected now.
    clients: u64,
    next_id: u64,
    /// The config the engine runs after live parameter edits, kept across sessions (a page reload
    /// or a stop after the last page leaves the engine as edited); `None` means the applied one.
    /// An Apply swaps the engine, so it clears this.
    engine_running: Option<kaanal_core::Config>,
}

struct Session {
    id: u64,
    feed: GeneratorHandle,
    /// Ticks and markers for the pages following this session.
    events: broadcast::Sender<Arc<LiveEvent>>,
    started: Instant,
    /// When the system this session runs on started (`Generation::started`): the mock clock.
    system_started: Option<std::time::Instant>,
    /// The components in topology (file) order: the order of every tick's `nodes`. The draft's JSON
    /// is alphabetical, so a page must map by these names, never by position in its own copy.
    node_names: Vec<String>,
    /// The config the engine runs now: the applied system, then each live parameter edit (the
    /// draft's parameters over the running structure, `edit.rs`).
    running: Option<kaanal_core::Config>,
    /// The traffic profile the generator runs; `None` when the system has none.
    profile: Option<String>,
    /// Why nothing is generated, when that is so (no profile, a route that does not exist).
    note: Option<String>,
}

impl Session {
    fn marker(&self, kind: &'static str, text: &str) {
        let _ = self.events.send(Arc::new(LiveEvent::Marker(Marker {
            at_ms: since(self.started),
            kind,
            text: text.to_string(),
        })));
    }
}

/// A connected stream client: the session lives while at least one exists (plus [`GRACE`]).
pub(crate) struct ClientGuard {
    st: Arc<AppState>,
}

/// Held while an Apply drains and swaps: the generator is paused for it and resumes after, unless
/// the Apply restarted the session.
pub(crate) struct ApplyHold<'a> {
    slot: &'a LiveSlot,
    session: Option<u64>,
}

/// The first traffic profile of `g` as a mix and a rate, or why there is none.
fn mix_of(g: &Generation) -> Result<(String, Vec<StreamShare>, f64), String> {
    let Some(name) = g.traffic.keys().next().cloned() else {
        return Err("the file has no traffic profile: add a traffic: section to run live".into());
    };
    let req = RunRequest {
        profile: Profile::Named(name.clone()),
        duration: None,
        warmup: None,
        seed: None,
        scenario: None,
    };
    let mut planned = to_sim(g, &req).map_err(|e| e.message)?;
    planned.fill_sizes(g);
    let rps: f64 = planned.cfg.streams.iter().map(|s| s.rate_per_s).sum();
    let mix = planned
        .cfg
        .streams
        .iter()
        .map(|s| StreamShare {
            endpoint_key: s.endpoint_key.clone(),
            share: s.rate_per_s,
            req_bytes: s.req_bytes,
            resp_bytes: s.resp_bytes,
        })
        .collect();
    Ok((name, mix, rps))
}

/// The config of the applied system, parsed from the applied text (`None` before the first Apply
/// of a blank start, or when it does not parse).
fn applied_config(st: &AppState) -> Option<kaanal_core::Config> {
    let text = st
        .draft
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .applied
        .clone();
    kaanal_core::Config::from_yaml(&text).ok()
}

fn start_session(st: &Arc<AppState>, id: u64, running: Option<kaanal_core::Config>) -> Session {
    let g = st.generation();
    let planned = g
        .as_deref()
        .map_or_else(|| Err("no system is running yet".to_string()), mix_of);
    let seed = g.as_deref().map_or(0, |g| g.seed) ^ id;
    let (mix, rps, profile, note) = match planned {
        Ok((name, mix, rps)) => (mix, rps, Some(name), None),
        // an idle generator keeps the session's shape (pause, rate, stop) uniform
        Err(note) => (Vec::new(), 0.0, None, Some(note)),
    };
    let (ftx, frx) = mpsc::unbounded_channel();
    let feed = generator::start(Arc::clone(st), mix, rps, seed, Limits::default(), Some(ftx));
    let (events, _) = broadcast::channel(256);
    let started = Instant::now();
    tokio::spawn(tick_loop(
        Arc::clone(st),
        feed.clone(),
        events.clone(),
        frx,
        started,
    ));
    Session {
        id,
        feed,
        events,
        started,
        system_started: g.as_deref().map(|g| g.started),
        node_names: g
            .as_deref()
            .map(|g| g.topo.nodes.iter().map(|n| n.name.clone()).collect())
            .unwrap_or_default(),
        running,
        profile,
        note,
    }
}

fn since(t0: Instant) -> u64 {
    ms_of(t0.elapsed())
}

fn ms_of(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// **Private.** Every tick: the requests that finished are summed with the engine's queues into
/// one [`Tick`] and broadcast; a chaos change on the control bus becomes a marker. Ends when the
/// generator is stopped (the session ended).
async fn tick_loop(
    st: Arc<AppState>,
    feed: GeneratorHandle,
    events: broadcast::Sender<Arc<LiveEvent>>,
    mut finished: mpsc::UnboundedReceiver<kaanal_core::replay::Finished>,
    started: Instant,
) {
    let mut bus = st.events.subscribe();
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // the aggregator belongs to one generation: a different topology starts a new one
    let mut agg: Option<(u64, Aggregator)> = None;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if feed.is_stopped() {
                    return;
                }
                let Some(g) = st.generation() else { continue };
                if agg.as_ref().is_none_or(|(id, _)| *id != g.id) {
                    agg = Some((g.id, Aggregator::new(&g.topo)));
                }
                let a = &mut agg.as_mut().expect("just set").1;
                while let Ok(f) = finished.try_recv() {
                    a.add(&f);
                }
                let loads = match &g.engine {
                    Some(e) => e.loads().await.unwrap_or_default(),
                    None => Vec::new(),
                };
                let s = feed.stats();
                let status = TickStatus {
                    rate: s.rate,
                    paused: s.paused,
                    overloaded: s.overloaded,
                    in_flight: s.in_flight,
                };
                let tick = a.take(
                    (since(started), ms_of(g.started.elapsed())),
                    &loads,
                    &g.topo,
                    status,
                );
                let _ = events.send(Arc::new(LiveEvent::Tick(Box::new(tick))));
            }
            msg = bus.recv() => match msg {
                Ok(BusEvent::Scenario { action, name, .. }) => {
                    let text = name.map_or_else(|| format!("chaos {action}"), |n| format!("{n}: {action}"));
                    let _ = events.send(Arc::new(LiveEvent::Marker(Marker {
                        at_ms: since(started),
                        kind: "chaos",
                        text,
                    })));
                }
                Err(broadcast::error::RecvError::Closed) => return,
                _ => {}
            },
        }
    }
}

impl LiveSlot {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A stream client connects. With `fresh` (a page load) or when no session exists, a new
    /// session starts and the old one stops; otherwise the client joins the running one.
    pub(crate) fn attach(&self, st: &Arc<AppState>, fresh: bool) -> ClientGuard {
        // parsed before the lock: the draft lock is never taken inside it
        let applied = applied_config(st);
        let mut g = self.lock();
        if fresh || g.session.is_none() {
            if let Some(old) = g.session.take() {
                old.feed.stop();
            }
            g.next_id += 1;
            let id = g.next_id;
            let running = g.engine_running.clone().or(applied);
            g.session = Some(start_session(st, id, running));
        }
        g.clients += 1;
        ClientGuard { st: Arc::clone(st) }
    }

    /// A receiver for the running session's ticks and markers; `None` when no session runs.
    pub(crate) fn subscribe(&self) -> Option<broadcast::Receiver<Arc<LiveEvent>>> {
        self.lock().session.as_ref().map(|s| s.events.subscribe())
    }

    /// Puts a mark on the running session's timeline (a parameter edit, a structure change).
    pub(crate) fn mark(&self, kind: &'static str, text: &str) {
        if let Some(s) = self.lock().session.as_ref() {
            s.marker(kind, text);
        }
    }

    /// The session's state as JSON, or `None` when no session runs.
    pub(crate) fn status(&self) -> Option<Value> {
        let g = self.lock();
        let s = g.session.as_ref()?;
        let st = s.feed.stats();
        Some(json!({
            "session": s.id,
            "profile": s.profile,
            "note": s.note,
            "rate": st.rate,
            "paused": st.paused,
            "overloaded": st.overloaded,
            "in_flight": st.in_flight,
            "submitted": st.submitted,
            "clients": g.clients,
            "system_ms": s.system_started.map(|t| ms_of(t.elapsed())),
            "nodes": s.node_names,
        }))
    }

    /// Pauses the generator.
    ///
    /// # Errors
    /// [`LiveError::NoSession`] when none is running.
    pub(crate) fn pause(&self) -> Result<(), LiveError> {
        self.lock()
            .session
            .as_ref()
            .map(|s| {
                s.feed.pause();
                s.marker("pause", "paused");
            })
            .ok_or(LiveError::NoSession)
    }

    /// Resumes the generator.
    ///
    /// # Errors
    /// [`LiveError::NoSession`] when none is running.
    pub(crate) fn resume(&self) -> Result<(), LiveError> {
        self.lock()
            .session
            .as_ref()
            .map(|s| {
                s.feed.resume();
                s.marker("resume", "resumed");
            })
            .ok_or(LiveError::NoSession)
    }

    /// Sets the offered rate.
    ///
    /// # Errors
    /// [`LiveError::NoSession`], or [`LiveError::Refused`] with the generator's reason.
    pub(crate) fn set_rate(&self, rps: f64) -> Result<(), LiveError> {
        let g = self.lock();
        let s = g.session.as_ref().ok_or(LiveError::NoSession)?;
        s.feed
            .set_rate(rps)
            .map_err(|RateRefused(m)| LiveError::Refused(m))
    }

    /// An Apply is about to drain and swap: pauses the generator for it. The hold resumes it when
    /// dropped, unless the session was restarted by then.
    pub(crate) fn hold_for_apply(&self) -> ApplyHold<'_> {
        let g = self.lock();
        let session = g.session.as_ref().and_then(|s| {
            let was_paused = s.feed.stats().paused;
            s.feed.pause();
            (!was_paused).then_some(s.id)
        });
        ApplyHold {
            slot: self,
            session,
        }
    }

    /// The system was swapped by an Apply: a running session restarts on the new engine (a fresh
    /// run, as the design says a structure change restarts it).
    pub(crate) fn restart(&self, st: &Arc<AppState>) {
        let running = applied_config(st);
        let mut g = self.lock();
        // a new engine runs the applied system, whether or not a page follows it now
        g.engine_running = None;
        let Some(old) = g.session.take() else { return };
        old.feed.stop();
        g.next_id += 1;
        let id = g.next_id;
        g.session = Some(start_session(st, id, running));
        if g.clients == 0 {
            // the last page left inside its grace: the timer that was set watches the old session
            schedule_stop(st, id);
        }
    }

    /// The config the engine runs now; `None` when no session runs.
    pub(crate) fn running(&self) -> Option<kaanal_core::Config> {
        self.lock().session.as_ref().and_then(|s| s.running.clone())
    }

    /// Records the config the engine runs after a live parameter edit.
    pub(crate) fn set_running(&self, cfg: kaanal_core::Config) {
        let mut g = self.lock();
        g.engine_running = Some(cfg.clone());
        if let Some(s) = g.session.as_mut() {
            s.running = Some(cfg);
        }
    }
}

/// Stops session `id` after [`GRACE`] if no page is connected by then and it is still the one
/// that runs (a page load or an Apply may have replaced it).
fn schedule_stop(st: &Arc<AppState>, id: u64) {
    let st = Arc::clone(st);
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move {
            tokio::time::sleep(GRACE).await;
            let mut g = st.live.lock();
            if g.clients == 0
                && g.session.as_ref().is_some_and(|s| s.id == id)
                && let Some(old) = g.session.take()
            {
                old.feed.stop();
            }
        });
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let current = {
            let mut g = self.st.live.lock();
            g.clients = g.clients.saturating_sub(1);
            // the session that runs now, not the one this page joined: an Apply may have restarted it
            (g.clients == 0)
                .then(|| g.session.as_ref().map(|s| s.id))
                .flatten()
        };
        if let Some(id) = current {
            schedule_stop(&self.st, id);
        }
    }
}

impl Drop for ApplyHold<'_> {
    fn drop(&mut self) {
        let Some(id) = self.session else { return };
        let g = self.slot.lock();
        if let Some(s) = g.session.as_ref().filter(|s| s.id == id) {
            s.feed.resume();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::Config;
    use kaanal_core::replay::Engine;

    const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
traffic:
  peak:
    rps: 100
    journeys:
      - { endpoint: 'GET /users/{id}', weight: 1 }
";

    fn state(yaml: &str) -> Arc<AppState> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let cfg = Config::from_yaml(yaml).unwrap();
        let mut st = AppState::build(&cfg, &dir, "t".into(), 0).unwrap();
        let g = st.gen_mut();
        g.engine = Some(crate::actor::spawn(Engine::live(&g.topo)));
        Arc::new(st)
    }

    fn running(st: &AppState) -> bool {
        st.live.status().is_some()
    }

    #[tokio::test(start_paused = true)]
    async fn the_first_client_starts_a_session_and_the_last_leaving_stops_it_after_5s() {
        let st = state(YAML);
        assert!(!running(&st));
        let guard = st.live.attach(&st, true);
        assert!(running(&st));
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(st.live.status().unwrap()["submitted"].as_u64().unwrap() > 50);
        drop(guard);
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(running(&st), "still inside the 5 s grace");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!running(&st), "no orphan session after the last client");
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_resets_and_an_automatic_reconnect_does_not() {
        let st = state(YAML);
        let g1 = st.live.attach(&st, true);
        let first = st.live.status().unwrap()["session"].as_u64().unwrap();
        // the same page reconnects (no fresh): the session and its run are kept
        drop(g1);
        let g2 = st.live.attach(&st, false);
        assert_eq!(
            st.live.status().unwrap()["session"].as_u64().unwrap(),
            first
        );
        // a page load says fresh: a new session, and the old grace timer must not stop the new one
        drop(g2);
        let g3 = st.live.attach(&st, true);
        let second = st.live.status().unwrap()["session"].as_u64().unwrap();
        assert_ne!(second, first);
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(
            st.live.status().unwrap()["session"].as_u64().unwrap(),
            second
        );
        drop(g3);
    }

    /// Review finding 4: an Apply restarts the session while the last page is gone; the grace
    /// timer must still stop the restarted one.
    #[tokio::test(start_paused = true)]
    async fn a_session_restarted_inside_the_grace_is_still_stopped() {
        let st = state(YAML);
        let guard = st.live.attach(&st, true);
        drop(guard);
        tokio::time::sleep(Duration::from_secs(1)).await;
        st.live.restart(&st);
        assert!(running(&st));
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(!running(&st), "no orphan after an Apply inside the grace");
    }

    /// Review finding 4, other order: the page leaves just after the restart.
    #[tokio::test(start_paused = true)]
    async fn the_last_page_leaving_after_a_restart_stops_the_new_session() {
        let st = state(YAML);
        let guard = st.live.attach(&st, true);
        st.live.restart(&st);
        drop(guard);
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(
            !running(&st),
            "the guard must watch the session that runs now"
        );
    }

    /// Review finding 6: a new session on the same engine keeps the running config, not the stale
    /// applied text, so a later edit compares against what the engine really runs.
    #[tokio::test(start_paused = true)]
    async fn a_new_session_keeps_the_config_the_engine_runs() {
        let st = state(YAML);
        let g1 = st.live.attach(&st, true);
        let edited = Config::from_yaml(&YAML.replace(
            "proc: 1ms }
  app",
            "proc: 9ms }
  app",
        ))
        .unwrap();
        st.live.set_running(edited.clone());
        drop(g1);
        let g2 = st.live.attach(&st, true);
        assert_eq!(
            format!("{:?}", st.live.running().unwrap()),
            format!("{edited:?}")
        );
        tokio::time::sleep(Duration::from_secs(10)).await;
        // after every page left and came back, still the engine's config
        drop(g2);
        tokio::time::sleep(Duration::from_secs(10)).await;
        let _g3 = st.live.attach(&st, true);
        assert_eq!(
            format!("{:?}", st.live.running().unwrap()),
            format!("{edited:?}")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn two_clients_share_one_session() {
        let st = state(YAML);
        let a = st.live.attach(&st, true);
        let b = st.live.attach(&st, false);
        assert_eq!(st.live.status().unwrap()["clients"], 2);
        drop(a);
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(running(&st), "one client still connected");
        drop(b);
    }

    #[tokio::test(start_paused = true)]
    async fn an_apply_drain_pauses_the_generator_and_resumes_after() {
        let st = state(YAML);
        let _c = st.live.attach(&st, true);
        tokio::time::sleep(Duration::from_secs(1)).await;
        {
            let _hold = st.live.hold_for_apply();
            tokio::time::sleep(Duration::from_millis(100)).await;
            let at = st.live.status().unwrap()["submitted"].as_u64().unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert_eq!(st.live.status().unwrap()["submitted"].as_u64().unwrap(), at);
            assert_eq!(st.live.status().unwrap()["paused"], true);
        }
        assert_eq!(
            st.live.status().unwrap()["paused"],
            false,
            "resumed after the apply"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_hold_does_not_resume_what_the_person_paused() {
        let st = state(YAML);
        let _c = st.live.attach(&st, true);
        st.live.pause().unwrap();
        drop(st.live.hold_for_apply());
        assert_eq!(st.live.status().unwrap()["paused"], true);
    }

    #[tokio::test(start_paused = true)]
    async fn commands_without_a_session_say_so_and_a_missing_profile_is_a_note() {
        let st = state(YAML);
        assert_eq!(st.live.pause(), Err(LiveError::NoSession));
        assert_eq!(st.live.set_rate(10.0), Err(LiveError::NoSession));
        let no_traffic = state(&YAML[..YAML.find("traffic:").unwrap()]);
        let _c = no_traffic.live.attach(&no_traffic, true);
        let s = no_traffic.live.status().unwrap();
        assert!(s["note"].as_str().unwrap().contains("traffic"), "{s}");
        assert_eq!(s["submitted"], 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_rate_is_reported_and_the_run_keeps_its_rate() {
        let st = state(YAML);
        let _c = st.live.attach(&st, true);
        assert!(matches!(st.live.set_rate(1e9), Err(LiveError::Refused(_))));
        assert_eq!(st.live.status().unwrap()["rate"], 100.0);
        assert_eq!(st.live.set_rate(250.0), Ok(()));
        assert_eq!(st.live.status().unwrap()["rate"], 250.0);
    }

    #[tokio::test(start_paused = true)]
    async fn ticks_carry_what_the_generator_did_and_markers_the_changes() {
        let st = state(YAML);
        let _c = st.live.attach(&st, true);
        let mut rx = st.live.subscribe().expect("a session");
        tokio::time::sleep(Duration::from_secs(1)).await;
        st.live.pause().unwrap();
        st.live.resume().unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let (mut ticks, mut requests, mut marks) = (0, 0u32, Vec::new());
        while let Ok(ev) = rx.try_recv() {
            match &*ev {
                LiveEvent::Tick(t) => {
                    ticks += 1;
                    requests += t.requests;
                    assert_eq!(t.nodes.len(), 3);
                    assert!((t.status.rate - 100.0).abs() < f64::EPSILON);
                }
                LiveEvent::Marker(m) => marks.push((m.kind, m.text.clone())),
            }
        }
        assert!(ticks >= 6, "{ticks} ticks in 1.6 s at 4 a second");
        assert!(requests > 80, "{requests} requests finished in the ticks");
        assert_eq!(
            marks,
            vec![
                ("pause", "paused".to_string()),
                ("resume", "resumed".to_string())
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_chaos_change_on_the_bus_becomes_a_marker() {
        let st = state(YAML);
        let _c = st.live.attach(&st, true);
        let mut rx = st.live.subscribe().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = st.events.send(BusEvent::Scenario {
            action: "activate",
            name: Some("lag".into()),
            chaos_key: None,
            active: vec!["lag".into()],
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut found = false;
        while let Ok(ev) = rx.try_recv() {
            if let LiveEvent::Marker(m) = &*ev {
                found |= m.kind == "chaos" && m.text.contains("lag");
            }
        }
        assert!(found, "no chaos marker");
    }

    #[tokio::test(start_paused = true)]
    async fn hello_names_the_components_in_the_order_of_every_tick() {
        let st = state(YAML);
        let _c = st.live.attach(&st, true);
        // file order (nginx, app, db), not alphabetical (app, db, nginx)
        assert_eq!(
            st.live.status().unwrap()["nodes"],
            serde_json::json!(["nginx", "app", "db"])
        );
        let mut rx = st.live.subscribe().unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        while let Ok(ev) = rx.try_recv() {
            if let LiveEvent::Tick(t) = &*ev {
                assert_eq!(t.nodes.len(), 3);
            }
        }
    }
}
