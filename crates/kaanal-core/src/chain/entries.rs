//! `Compiler` methods that list the endpoints to compile and their jobs, built-in and user.

use super::{
    BTreeMap, Compiler, EndpointsInput, Entry, IndexMap, JWKS_KEY, Job, LOGIN_KEY, Located,
    METHODS, NodeId, Op, Path, PathSeg, Place, Responder, RouteSpec, collection_name_ok,
    collection_op, error_key,
};

impl Compiler<'_> {
    /// **Private.** Checks the key and values of every `endpoints:` entry, keyed by the
    /// normalized route, in file order.
    pub(super) fn entries<'e>(
        &self,
        input: EndpointsInput<'e>,
    ) -> Result<IndexMap<String, Entry<'e>>, Located> {
        let topo = self.topo;
        let prefix = if input.file.is_none() {
            vec![PathSeg::Key("endpoints".into())]
        } else {
            Vec::new()
        };
        let file = input.file.map(Path::to_path_buf);
        let mut entries: IndexMap<String, Entry<'e>> = IndexMap::new();
        let mut written_as: BTreeMap<String, String> = BTreeMap::new();
        for (written, cfg) in input.entries.into_iter().flatten() {
            let mut path = prefix.clone();
            path.push(PathSeg::Key(written.clone()));
            let place = Place {
                ctx: format!("endpoint '{written}'"),
                path,
                file: file.clone(),
            };
            let Some((m, p)) = written.split_once(' ') else {
                return Err(place.err("key must look like 'GET /path'"));
            };
            let m = m.to_ascii_uppercase();
            if !METHODS.contains(&m.as_str()) {
                return Err(place.err(format!("unknown method '{m}'")));
            }
            let p = p.trim();
            if !p.starts_with('/') {
                return Err(place.err("path must start with '/'"));
            }
            // The checks above make this infallible; one normalizer for every endpoint key.
            let key = crate::scenario::endpoint_key(written).map_err(|e| place.err(e))?;
            if let Some(first) = written_as.get(&key) {
                return Err(place.err(format!("same route as '{first}'")));
            }
            written_as.insert(key.clone(), written.clone());
            if let Some(s) = cfg.response.as_ref().map(|o| o.status)
                && !(200..=599).contains(&s)
            {
                return Err(place
                    .key("override")
                    .key("status")
                    .bad(format!("override: status {s} must be between 200 and 599")));
            }
            if let Some(o) = &cfg.response {
                o.check_headers()
                    .and_then(|()| o.check_content_type())
                    .map_err(|m| place.key("override").bad(format!("override: {m}")))?;
            }
            // Every entry, live or stale, so a typo in `states:` is always caught.
            Self::states(Some(cfg), &place)?;
            if let Some(name) = &cfg.collection {
                if !collection_name_ok(name) {
                    return Err(place.err(format!(
                        "collection: name '{name}' must be 1-32 characters of a-z, 0-9, '_' or '-', starting with a letter"
                    )));
                }
                if collection_op(&m, p).is_none() {
                    return Err(place
                        .err("collection: needs GET /x, GET /x/{id}, POST /x or DELETE /x/{id}"));
                }
                if cfg.response.is_some() {
                    return Err(place.err(
                        "collection: cannot be used with override (override replaces the responder)",
                    ));
                }
            }
            Self::total_conflict(cfg, &place)?;
            let mut error_map = IndexMap::new();
            let mut written_keys: BTreeMap<String, String> = BTreeMap::new();
            for (k, v) in &cfg.error_map {
                let at = place.key("error_map").key(k);
                let Some(norm) = error_key(k, topo) else {
                    return Err(at.err(format!(
                        "error_map: '{k}' is not a node name, a status (400-599) or a class (4xx, 5xx)"
                    )));
                };
                if !(200..=599).contains(&v.status) {
                    return Err(at.bad(format!(
                        "error_map: status {} must be between 200 and 599",
                        v.status
                    )));
                }
                if let Some(first) = written_keys.get(&norm) {
                    return Err(at.err(format!("error_map: '{k}' is the same key as '{first}'")));
                }
                written_keys.insert(norm.clone(), k.clone());
                error_map.insert(norm, v.clone());
            }
            entries.insert(
                key,
                Entry {
                    written: written.clone(),
                    method: m,
                    path: p.to_string(),
                    cfg,
                    error_map,
                    place,
                },
            );
        }
        Ok(entries)
    }

    /// **Private.** `total:` sets the size of a generated list, so it cannot meet a fixed
    /// response or a collection.
    fn total_conflict(cfg: &crate::config::EndpointConfig, place: &Place) -> Result<(), Located> {
        if cfg.total.is_none() {
            return Ok(());
        }
        for (other, set) in [
            ("override", cfg.response.is_some()),
            ("collection", cfg.collection.is_some()),
        ] {
            if set {
                return Err(place.err(format!("`total` cannot be combined with `{other}`")));
            }
        }
        Ok(())
    }

    /// **Private.** Lists the endpoints to compile: spec routes in order (with their entry,
    /// if any), then the auth node's built-in routes, then `HEAD` entries of a `GET` route,
    /// then static entries. Stale entries become warnings.
    pub(super) fn jobs<'e>(
        &mut self,
        routes: &[RouteSpec],
        input: EndpointsInput<'e>,
    ) -> Result<Vec<Job<'e>>, Located> {
        let topo = self.topo;
        let mut entries = self.entries(input)?;
        let mut jobs = Vec::new();
        for r in routes {
            let job = match entries.shift_remove(&r.key) {
                Some(e) => Job {
                    key: r.key.clone(),
                    owner: Some(r.owner),
                    op: r.op,
                    cfg: Some(e.cfg),
                    error_map: e.error_map,
                    place: e.place,
                    responder: None,
                },
                None => Job {
                    key: r.key.clone(),
                    owner: Some(r.owner),
                    op: r.op,
                    cfg: None,
                    error_map: IndexMap::new(),
                    // No entry to point at: a limit hit by the default chain is placed at the
                    // owning node, whose edges (`calls`) decide its size.
                    place: Place {
                        ctx: format!("endpoint '{}'", r.key),
                        path: vec![
                            PathSeg::Key("nodes".into()),
                            PathSeg::Key(topo.nodes[r.owner].name.clone()),
                        ],
                        file: None,
                    },
                    responder: None,
                },
            };
            jobs.push(job);
        }
        self.builtin_jobs(routes, &mut entries, &mut jobs);
        // GET entries with an override: a HEAD entry for one is served as a copy of it.
        let static_gets: Vec<String> = entries
            .iter()
            .filter(|(k, e)| k.starts_with("GET ") && e.cfg.response.is_some())
            .map(|(k, _)| k.clone())
            .collect();
        for (key, e) in entries {
            let get = (e.method == "HEAD")
                .then(|| routes.iter().find(|r| r.key == format!("GET {}", e.path)))
                .flatten();
            // A HEAD entry for the built-in JWKS route is a copy of it, like one for a spec GET.
            let builtin = (e.method == "HEAD" && format!("GET {}", e.path) == JWKS_KEY)
                .then_some(topo.auth)
                .flatten();
            let mut responder = None;
            let (owner, op) = if let Some(r) = get {
                (Some(r.owner), r.op)
            } else if let Some(auth) = builtin {
                responder = Some(Responder::Jwks);
                (Some(auth), Op::Read)
            } else if e.cfg.response.is_none()
                && static_gets.contains(&format!("GET {}", e.path))
                && e.method == "HEAD"
            {
                self.warnings.push(format!(
                    "endpoint '{}': ignored: a HEAD of a static GET entry is served as a copy of that GET (give it an override to serve it separately)",
                    e.written
                ));
                continue;
            } else if e.cfg.response.is_none() {
                self.warnings.push(format!(
                    "endpoint '{}': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)",
                    e.written
                ));
                continue;
            } else if topo.gateway.is_none() {
                return Err(e
                    .place
                    .err("static route needs a gateway or a service that owns it"));
            } else {
                (None, Op::infer(&e.method, &e.path))
            };
            jobs.push(Job {
                key,
                owner,
                op,
                cfg: Some(e.cfg),
                error_map: e.error_map,
                place: e.place,
                responder,
            });
        }
        Ok(jobs)
    }

    /// **Private.** X7, X9: the auth node's built-in routes, listed before stale detection so
    /// an `endpoints:` entry customises them; a spec route with the same key replaces one (P8).
    pub(super) fn builtin_jobs<'e>(
        &mut self,
        routes: &[RouteSpec],
        entries: &mut IndexMap<String, Entry<'e>>,
        jobs: &mut Vec<Job<'e>>,
    ) {
        let topo = self.topo;
        let Some(auth) = topo.auth else { return };
        for (key, responder) in [(LOGIN_KEY, Responder::Login), (JWKS_KEY, Responder::Jwks)] {
            if let Some(r) = routes.iter().find(|r| r.key == key) {
                self.warnings.push(format!(
                    "endpoint '{key}': the spec of '{}' defines it, so it replaces the built-in route of auth node '{}'",
                    topo.nodes[r.owner].name, topo.nodes[auth].name
                ));
                continue;
            }
            let (cfg, error_map, place) = match entries.shift_remove(key) {
                Some(e) => (Some(e.cfg), e.error_map, e.place),
                None => (
                    None,
                    IndexMap::new(),
                    Place {
                        ctx: format!("endpoint '{key}'"),
                        path: vec![
                            PathSeg::Key("nodes".into()),
                            PathSeg::Key(topo.nodes[auth].name.clone()),
                        ],
                        file: None,
                    },
                ),
            };
            jobs.push(Job {
                key: key.to_string(),
                owner: Some(auth),
                // X12: both built-in routes read.
                op: Op::Read,
                cfg,
                error_map,
                place,
                responder: Some(responder),
            });
        }
    }

    /// **Private.** `"chains: a, b"` or `"it has no chains"`.
    pub(super) fn chain_names(&self, n: NodeId) -> String {
        let node = &self.topo.nodes[n];
        if node.chains.is_empty() {
            "it has no chains".to_string()
        } else {
            let names: Vec<&str> = node.chains.keys().map(String::as_str).collect();
            format!("chains: {}", names.join(", "))
        }
    }

    /// **Private.** `"endpoints: K1, K2"` or `"it has none"` (sorted, no HEAD copies).
    pub(super) fn endpoint_names(&self, n: NodeId) -> String {
        let keys: Vec<&str> = self
            .done
            .values()
            .filter(|e| e.owner == Some(n) && !e.synthesized)
            .map(|e| e.key.as_str())
            .collect();
        if keys.is_empty() {
            "it has none".to_string()
        } else {
            format!("endpoints: {}", keys.join(", "))
        }
    }
}
