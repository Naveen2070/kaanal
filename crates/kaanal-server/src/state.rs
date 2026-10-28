//! [`AppState`]: the stable shell of the running server (token, port, log, ids, events, chaos,
//! shutdown, collections), built once at startup. What the config builds (topology, routes,
//! endpoints, engine) is a [`Generation`], held behind a lock and handed out one `Arc` at a time
//! (see [`AppState::generation`]); the load pipeline is [`Generation::build`], shared by `serve`
//! and `check` (`kaanal validate`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use kaanal_core::config::SeedSource;
use kaanal_core::diagnostics::Located;
use kaanal_core::{ChaosStack, Config, EffectiveChaos, EffectivePins, Endpoint, ScenarioSet};

use crate::generation::{Generation, Trust};
use kaanal_data::PageSpec;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

/// One route: a `method + path` pair and the compiled endpoint that answers it.
pub struct Route {
    /// Uppercase HTTP method.
    pub method: String,
    /// `"GET /users/{id}"`: chain seed material and display name.
    pub key: String,
    /// The compiled chain, op, `override` and `error_map` (HEAD copies included, R6.8).
    pub endpoint: Arc<Endpoint>,
    /// Success status from the spec (200 for a static route); an `override` status wins.
    pub status: u16,
    /// Response schema for generated bodies (`None`: empty body, or a static route).
    pub schema: Option<Value>,
    /// Requests seen so far; becomes `ChainRequest.n`.
    pub counter: Arc<AtomicU64>,
    /// D3: the paging contract of a generated list route; `None` for every other route
    /// (unpaged, `override`, `collection:` or a built-in responder), which answers as before.
    pub paging: Option<Arc<PageSpec>>,
    /// D3: the `total:` of the route's entry; a `HEAD` that follows its `GET` and sets none takes
    /// the `GET`'s. `None` = seeded by the route.
    pub total: Option<u32>,
}

/// One finished request, broadcast to `/__kaanal/events` subscribers.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    /// Our request id (`r04f2-0042`).
    pub id: String,
    /// The client's `X-Request-Id`, when it matched the grammar.
    pub client_id: Option<String>,
    /// Arrival order, the id's number.
    pub seq: u64,
    /// Why the request was refused before any chain ran (`unmatched`, `bad-scope`, `engine-stopped`),
    /// or `client-gone` when the client left before the answer.
    pub rejected: Option<&'static str>,
    /// Unix timestamp in milliseconds.
    pub ts_ms: u64,
    /// Uppercase HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// The matched route template (`GET /users/{id}`); `None` when unmatched.
    pub endpoint: Option<String>,
    /// Response status.
    pub status: u16,
    /// Measured wall-clock milliseconds (not the simulated sum — see the trace header).
    pub ms: u64,
    /// Nodes this request touched, in order.
    pub nodes: Vec<String>,
    /// The node whose chaos failed the request, if any.
    pub failed_node: Option<String>,
    /// The UI state a pin forced on this request (`loading`, `empty`, `error`, `slow` or `success`).
    pub pin: Option<&'static str>,
    /// How it failed when not a plain status (`reset`, `malformed` or `timeout`); `None` for a
    /// status failure or a success. A reset or cut-off body answers 200, so this is how a client tells.
    pub kind: Option<&'static str>,
    /// The request's scenario scope: `None` for the global stack, `Some(key)` for an
    /// `X-Kaanal-Scenario` stack (the names joined by `,`), `Some("")` for an empty header.
    pub scope: Option<String>,
}

/// What the bus carries: a finished request, or a scenario change (S21).
// limitation: `Request` is the large variant (cloned once per subscriber, up to 1024 in the ring);
// `Arc<Event>` is the upgrade path if load shows a cost. Since `Scenario` gained `chaos_key` the
// size gap is under clippy's `large_enum_variant` threshold.
#[derive(Debug, Clone)]
pub enum BusEvent {
    /// A new system went live (SSE message `applied`).
    Applied {
        /// The generation now in force.
        generation: u64,
    },
    /// A finished mock request (SSE message `request`).
    Request(Event),
    /// A scenario stack change (SSE message `scenario`), or a live chaos lease that ran out.
    Scenario {
        /// `activate`, `deactivate`, `set`, `clear`, `reset`, `expire` (a `ttl` ran out) or `update` (an active scenario's definition changed).
        action: &'static str,
        /// The scenario concerned, when one.
        name: Option<String>,
        /// For a live chaos expiry only: the key whose leased units were cleared, as an object
        /// with the parts it has (`{"node":"db"}`, `{"endpoint":"GET /x"}` or both, the names
        /// of the `PUT /chaos` body); `name` is `None`.
        chaos_key: Option<Value>,
        /// The active stack after the change, bottom to top.
        active: Vec<String>,
    },
}

/// One published chaos state: the stack it was compiled from and the compiled result. Immutable
/// once published; a request clones the `Arc` and uses that one value (S10).
pub struct Effective {
    /// The active scenarios and the live layer, as they were when this was compiled.
    pub stack: ChaosStack,
    /// What the executor reads.
    pub chaos: EffectiveChaos,
    /// The pins in force, folded from the same stack.
    pub pins: EffectivePins,
    /// When each leased activation expires (a copy of [`Control::leases`]).
    pub leases: HashMap<String, tokio::time::Instant>,
    /// When each leased live chaos unit expires (a copy of [`Control::chaos_leases`]).
    pub chaos_leases: crate::chaos_lease::Leases,
}

/// The mutable side of chaos. Behind [`AppState::control`]: the only writer path (S11).
#[derive(Clone, Default)]
pub struct Control {
    /// Every defined scenario.
    pub scenarios: ScenarioSet,
    /// The active scenarios and the live layer.
    pub stack: ChaosStack,
    /// Activations with a `ttl`: scenario name to deadline. Only active names keep one
    /// (`commit` drops the rest).
    pub leases: HashMap<String, tokio::time::Instant>,
    /// Live chaos set with a `ttl`: (key, unit) to deadline. Only units the live layer still
    /// sets keep one (`commit` drops the rest).
    pub chaos_leases: crate::chaos_lease::Leases,
}

/// Shared server state; one `Arc` clones into every handler.
pub struct AppState {
    /// What the config built: topology, routes, endpoints, engine, auth. A request clones the `Arc`
    /// once ([`AppState::generation`], or the [`crate::generation::Gen`] extractor) and uses that one
    /// value, so a later swap never mixes two generations in one request. `None` only before the
    /// first Apply of a blank start.
    generation: RwLock<Option<Arc<Generation>>>,
    /// The published chaos state; readers clone the `Arc` and drop the lock ([`AppState::snapshot`]).
    pub chaos: RwLock<Arc<Effective>>,
    /// The scenarios and the stack. Every control write locks it, mutates, and calls
    /// [`AppState::publish`] before unlocking. Never held across an `.await`.
    pub control: Mutex<Control>,
    /// Woken when a scenario or chaos lease is added or replaced, so the lease task re-reads the
    /// earliest deadline of both tables.
    pub lease_wake: tokio::sync::Notify,
    /// Compiled scoped stacks by key (see [`AppState::scoped`]). Lock order: `control`, then this.
    /// [`AppState::publish`] clears it on every publish.
    pub scoped: RwLock<HashMap<String, Arc<crate::scope::Scoped>>>,
    /// Event bus for the SSE stream (capacity 1024; slow subscribers get `lagged`).
    pub events: broadcast::Sender<BusEvent>,
    /// Bearer token the control API requires.
    pub token: String,
    /// The startup draw under `determinism: realistic`, printed by `up` for replay. It outlives
    /// every generation: whoever builds the next one (Apply) must pass this to
    /// [`Generation::build`], or a realistic config silently falls back to `seed:`.
    pub drawn_seed: Option<u64>,
    /// The bound port (after the +20 search), used by the Host guard.
    pub port: u16,
    /// Holds mock traffic back while an Apply drains and swaps (see [`crate::draft::gate`]).
    pub gate: crate::draft::gate::Gate,
    /// One Apply at a time.
    pub apply_lock: tokio::sync::Mutex<()>,
    /// One Save at a time (its disk work runs without the `draft` lock held).
    pub save_lock: Mutex<()>,
    /// The signing key an auth node needs, kept so an Apply can build the new issuer.
    pub auth_key: Option<crate::auth::AuthKey>,
    /// The directory the config was loaded from: spec, data and scenario files resolve against it.
    pub dir: std::path::PathBuf,
    /// The Studio draft: the `kaanal.yaml` text being edited, apart from the running system.
    pub draft: Mutex<crate::draft::Draft>,
    /// Hands out request ids; never reset.
    pub ids: crate::log::IdGen,
    /// The last finished requests. Push only through [`AppState::push_log`].
    pub log: Mutex<crate::log::LogRing>,
    /// The graceful stop: `POST /__kaanal/shutdown` and `BoundServer::shutdown` ask it.
    pub shutdown: crate::shutdown::Shutdown,
    /// The `collection:` items; shared by every scope and user, emptied by `POST /reset`.
    pub collections: crate::collections::Collections,
    /// Simulation runs started from Studio (one at a time, the last eight kept).
    pub sims: crate::sim::SimStore,
    /// Permits for CPU-heavy control work outside a run (`/sample`, `/openapi/skeleton`): a few
    /// at once, so repeated calls cannot fill the blocking pool the mocks' neighbours share.
    pub work: Arc<tokio::sync::Semaphore>,
}

impl AppState {
    /// The published chaos state. One `Arc` clone; the lock is gone on return.
    pub fn snapshot(&self) -> Arc<Effective> {
        Arc::clone(&self.chaos.read().expect("chaos lock poisoned"))
    }

    /// Records a finished request and returns its `done`. The lock is short and never held across
    /// an `.await`; a poisoned lock is used as is (the ring holds plain data).
    pub fn push_log(&self, e: crate::log::LogEntry) -> u64 {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(e)
    }

    /// Compiles `c` and swaps the result in; returns what was published. Taking the guard
    /// proves the caller holds the [`Control`] lock, so publishes happen in write order (S11).
    ///
    /// # Errors
    /// The compiled result breaks a chaos limit (a fault rate outside 0..=1 or a status outside
    /// 500-504). Each layer is validated on entry and latency is clamped by the compile, so
    /// this cannot happen today; it guards the invariant. Nothing is swapped.
    pub fn publish(&self, c: &MutexGuard<'_, Control>) -> Result<Arc<Effective>, String> {
        let chaos = c.stack.compile(&c.scenarios);
        for (_, r) in chaos.entries() {
            for f in r.faults() {
                // The union folds in floats, so allow a hair of rounding above 1.
                if !(0.0..=1.0 + 1e-9).contains(&f.upto) || !(500..=504).contains(&f.status) {
                    return Err(
                        "the combined chaos is out of range (error_rate 0-1, error_status 500-504)"
                            .into(),
                    );
                }
            }
        }
        let next = Arc::new(Effective {
            stack: c.stack.clone(),
            chaos,
            pins: c.stack.compile_pins(&c.scenarios),
            leases: c.leases.clone(),
            chaos_leases: c.chaos_leases.clone(),
        });
        *self.chaos.write().expect("chaos lock poisoned") = Arc::clone(&next);
        // Any definition or stack change may alter a scoped stack; the caller holds `control`.
        self.scoped.write().expect("scoped lock poisoned").clear();
        Ok(next)
    }

    /// Runs the full load pipeline: topology → spec files → endpoints (inline or file) →
    /// compiled routes → matchit router.
    ///
    /// Spec paths are resolved against `base_dir` (the config file's directory).
    /// Routes are inserted in deterministic (`BTreeMap`) order so a conflict always
    /// names the same path.
    ///
    /// # Errors
    /// Returns a [`Located`] error (path into `kaanal.yaml` + message, with file paths
    /// where relevant) for unreadable or invalid specs or endpoints file, route conflicts,
    /// any `endpoints:` error from `compile` (placed in the endpoints file when it comes
    /// from there), or any topology error. Nothing is served half-configured (rule R6.1).
    pub fn build(
        cfg: &Config,
        base_dir: &Path,
        token: String,
        port: u16,
    ) -> Result<AppState, Located> {
        let drawn_seed = match cfg.seed_source() {
            SeedSource::Fixed(_) => None,
            // R2.3: entropy is the server's job, drawn once here; no RNG state is kept.
            SeedSource::Entropy => Some(rand::random::<u64>()),
        };
        let (g, scenarios) = Generation::build(cfg, base_dir, drawn_seed, Trust::Owner)?;
        Ok(Self::assemble(
            Some(g),
            scenarios,
            drawn_seed,
            base_dir,
            token,
            port,
        ))
    }

    /// A state with no system yet (a blank start): the control API and the Studio draft work,
    /// the mock answers 404, and the first Apply makes the first generation. The startup seed
    /// is drawn now, because `drawn_seed` outlives every generation: a realistic config applied
    /// later must find one (see [`AppState::drawn_seed`]).
    pub fn blank(base_dir: &Path, token: String, port: u16) -> AppState {
        let drawn = Some(rand::random::<u64>());
        Self::assemble(None, ScenarioSet::default(), drawn, base_dir, token, port)
    }

    /// **Private.** The state around an optional first generation.
    fn assemble(
        g: Option<Generation>,
        scenarios: ScenarioSet,
        drawn_seed: Option<u64>,
        base_dir: &Path,
        token: String,
        port: u16,
    ) -> AppState {
        let (events, _) = broadcast::channel(1024);
        let ids = crate::log::IdGen::new();
        let log = Mutex::new(crate::log::LogRing::new(ids.boot()));
        AppState {
            generation: RwLock::new(g.map(Arc::new)),
            chaos: RwLock::new(Arc::new(Effective {
                stack: ChaosStack::default(),
                chaos: EffectiveChaos::default(),
                pins: EffectivePins::default(),
                leases: HashMap::new(),
                chaos_leases: HashMap::new(),
            })),
            control: Mutex::new(Control {
                scenarios,
                stack: ChaosStack::default(),
                leases: HashMap::new(),
                chaos_leases: HashMap::new(),
            }),
            lease_wake: tokio::sync::Notify::new(),
            scoped: RwLock::new(HashMap::new()),
            events,
            token,
            drawn_seed,
            port,
            collections: crate::collections::Collections::new(),
            sims: crate::sim::SimStore::default(),
            work: Arc::new(tokio::sync::Semaphore::new(crate::sim::WORK_PERMITS)),
            gate: crate::draft::gate::Gate::default(),
            apply_lock: tokio::sync::Mutex::new(()),
            save_lock: Mutex::new(()),
            auth_key: None,
            dir: base_dir.to_path_buf(),
            draft: Mutex::new(crate::draft::Draft::default()),
            ids,
            log,
            shutdown: crate::shutdown::Shutdown::new(),
        }
    }

    /// The generation in force, one `Arc` clone (the lock is gone on return). A request calls
    /// this once and reads everything from the result, so a swap cannot split it across two
    /// generations. `None` before the first Apply of a blank start.
    pub fn generation(&self) -> Option<Arc<Generation>> {
        self.generation
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the generation in force (or empties it, as a blank start has none).
    #[cfg(test)]
    pub(crate) fn set_generation(&self, g: Option<Arc<Generation>>) {
        *self
            .generation
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = g;
    }

    /// Puts `g` in force, as the next generation. The caller holds the `control` lock and has
    /// drained the gate, so no request straddles the change.
    pub(crate) fn install(&self, g: Generation) {
        *self
            .generation
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(g));
    }

    /// The generation, to set what a plain `build` leaves out (the engine and the issuer, which
    /// need a runtime and a key). Only before the state is shared: no clone of the `Arc` may exist.
    ///
    /// # Panics
    /// If there is no generation, or one is held elsewhere.
    pub fn gen_mut(&mut self) -> &mut Generation {
        let slot = self
            .generation
            .get_mut()
            .expect("generation lock poisoned")
            .as_mut()
            .expect("no generation installed");
        Arc::get_mut(slot).expect("generation is shared; set the engine before sharing the state")
    }
}
