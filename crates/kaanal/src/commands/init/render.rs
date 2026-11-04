//! What `kaanal init` writes: the server style, the `kaanal.yaml` text and the one-line summary.
//! Pure functions, no I/O. The numbers are the Studio starter numbers (`kaanal_core::presets::
//! starter`; a unit test keeps them equal) and are estimates, labelled so in the file (RULES 57).

use std::fmt::Write as _;

use super::{Params, SCHEMA_HEADER};

/// What a slot of the app service covers (F2). The answer to the "Server style" question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Node, Go, async Python: a slot is held only while the service does its own work.
    EventLoop,
    /// Tomcat, sync Gunicorn, Puma: a slot is held until everything the service called answers.
    ThreadPerRequest,
}

impl Style {
    /// The two words a user may type or pass to `--server-style`.
    pub const WORDS: &str = "event-loop or thread-per-request";

    /// **Private.** The style answer: exactly one of the two words.
    pub fn parse(s: &str) -> Result<Style, String> {
        match s {
            "event-loop" => Ok(Style::EventLoop),
            "thread-per-request" => Ok(Style::ThreadPerRequest),
            _ => Err(format!(
                "server style '{}' must be {}",
                s.escape_debug(),
                Style::WORDS
            )),
        }
    }

    /// Slots on the app service: 32 (the Studio starter), or 200 for a thread per request.
    fn slots(self) -> u32 {
        match self {
            Style::EventLoop => 32,
            Style::ThreadPerRequest => 200,
        }
    }

    /// **Private.** The app node's flow-style YAML (without the `spec`).
    fn app_fields(self) -> String {
        let hold = match self {
            Style::EventLoop => "",
            Style::ThreadPerRequest => ", hold: call",
        };
        format!(
            "concurrency: {{ limit: {}, queue: 100 }}, proc: 25ms{hold}",
            self.slots()
        )
    }

    /// The honesty line printed after `Wrote ...` (text mode only).
    pub fn summary(self) -> String {
        let how = match self {
            Style::EventLoop => "event loop",
            Style::ThreadPerRequest => "thread per request",
        };
        format!(
            "Chose: {how}, {} slots, queue 100, 30 s timeout (estimates; edit kaanal.yaml)",
            self.slots()
        )
    }
}

/// **Private.** One `endpoints:` line for a spec route key (`"GET /users/{id}"`), by method: a GET
/// whose path (trailing slashes ignored) ends in `{param}` is a `read` through the cache then the
/// database, any other GET a `scan` through the database, POST, PUT, PATCH and DELETE a `write`
/// to the database. Any other method (HEAD follows its GET, OPTIONS has no work of its own) gets
/// no entry, so the default chain applies. The key is a JSON string, a valid YAML quoted key.
fn endpoint_line(key: &str) -> Option<String> {
    let (method, path) = key.split_once(' ')?;
    let body = match method {
        "GET" if path.trim_end_matches('/').ends_with('}') => {
            "{ op: read, chain: [{ cache: cache }, { call: database }] }"
        }
        "GET" => "{ op: scan, chain: [{ call: database }] }",
        "POST" | "PUT" | "PATCH" | "DELETE" => "{ op: write, chain: [{ call: database }] }",
        _ => return None,
    };
    Some(format!(
        "  {}: {body}\n",
        serde_json::Value::String(key.into())
    ))
}

/// Most journeys a profile may have (`kaanal_core` checks it); a spec with more routes gets the
/// first of them in the profile, all of them in `endpoints:`.
const MAX_JOURNEYS: usize = 64;

/// **Private.** The `traffic:` section (RULES 58): one profile `peak`, 100 rps for 30 s, one
/// journey per route [`endpoint_line`] covers, weight 3 for a GET and 1 for a write. Empty when
/// there is no such route, because a profile needs at least one journey.
fn traffic_section(routes: &[String]) -> String {
    let mut lines = String::new();
    for k in routes
        .iter()
        .filter(|k| endpoint_line(k).is_some())
        .take(MAX_JOURNEYS)
    {
        let weight = if k.starts_with("GET ") { 3 } else { 1 };
        let key = serde_json::Value::String(k.clone());
        let _ = writeln!(lines, "      - {{ endpoint: {key}, weight: {weight} }}");
    }
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "# traffic: the load `kaanal sim run -p peak` replays; reads are three times as likely as writes. An estimate (edit freely)\n\
         traffic:\n\x20 peak:\n\x20   rps: 100\n\x20   duration: 30s\n\x20   journeys:\n{lines}"
    )
}

/// The generated `kaanal.yaml`: gateway to one service to a database and a cache, with the Studio
/// starter timings, a bounded queue on both, a net time on every edge, flavor presets for the
/// database and cache, and (from `routes`, the spec's route keys) one `endpoints:` entry per
/// route [`endpoint_line`] covers, then a `peak` traffic profile over them
/// ([`traffic_section`]). The spec path goes in as a JSON string, which is a valid YAML
/// double-quoted scalar for any path.
pub fn render_config(p: &Params, routes: &[String]) -> String {
    let spec = serde_json::to_string(&p.spec).unwrap_or_default();
    let (svc, port, locale) = (&p.service, p.port, &p.locale);
    let app = p.style.app_fields();
    let lines: String = routes.iter().filter_map(|k| endpoint_line(k)).collect();
    let endpoints = if lines.is_empty() {
        String::new()
    } else {
        format!(
            "# endpoints: what each route does behind the service; an estimate by method (edit freely)\n\
             endpoints:\n{lines}"
        )
    };
    let traffic = traffic_section(routes);
    format!(
        "{SCHEMA_HEADER}\n\
         version: 1\n\
         port: {port}\n\
         locale: {locale}\n\
         # Every number below is an estimate for a small deployment: slots, queues, times, the 30 s\n\
         # client timeout (clients and proxies usually give up after 30 s to 60 s). Edit freely.\n\
         nodes:\n\
         \x20 gateway: {{ type: gateway, flavor: nginx, concurrency: {{ limit: 256, queue: 100 }}, proc: 2ms, timeout: 30s }}\n\
         \x20 {svc}: {{ type: service, spec: {spec}, {app} }}\n\
         \x20 database: {{ type: database, flavor: postgres }}\n\
         \x20 cache: {{ type: cache, flavor: redis }}\n\
         edges:\n\
         \x20 - {{ from: gateway, to: {svc}, net: {{ p50: 1ms, p99: 3ms }} }}\n\
         \x20 - {{ from: {svc}, to: database, net: {{ p50: 1ms, p99: 3ms }} }}\n\
         \x20 - {{ from: {svc}, to: cache, net: {{ p50: 1ms, p99: 3ms }} }}\n\
         {endpoints}{traffic}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kaanal_core::config::{Config, NodeConfig};
    use kaanal_core::{NodeKind, presets};

    fn params(style: Style) -> Params {
        Params {
            service: "app".into(),
            port: 9000,
            locale: "en-US".into(),
            spec: "openapi.yaml".into(),
            style,
        }
    }

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn render_config_is_a_valid_project() {
        let text = render_config(&params(Style::EventLoop), &[]);
        assert!(text.starts_with(SCHEMA_HEADER), "{text}");
        assert!(
            !text.contains("Project") && !text.contains("hold"),
            "{text}"
        );
        assert!(text.contains("locale: en-US") && text.contains("port: 9000"));
        let cfg = Config::from_yaml(&text).unwrap();
        let kinds: Vec<_> = cfg.nodes.keys().map(String::as_str).collect();
        assert_eq!(kinds, ["gateway", "app", "database", "cache"]);
        assert!(matches!(cfg.nodes["gateway"], NodeConfig::Gateway(_)));
        // F3: the entry node carries the 30 s client timeout, labelled as an estimate
        assert_eq!(
            cfg.nodes["gateway"].timeout(),
            Some(std::time::Duration::from_secs(30))
        );
        assert!(text.contains("estimate"), "{text}");
        let NodeConfig::Service(s) = &cfg.nodes["app"] else {
            panic!("app is not a service")
        };
        assert_eq!(s.spec.as_deref(), Some("openapi.yaml"));
        assert_eq!(cfg.nodes["database"].flavor(), Some("postgres"));
        assert_eq!(cfg.nodes["cache"].flavor(), Some("redis"));
        let edges: Vec<_> = cfg.edges.iter().map(|e| (&*e.from, &*e.to)).collect();
        assert_eq!(
            edges,
            [("gateway", "app"), ("app", "database"), ("app", "cache")]
        );
        assert!(cfg.edges.iter().all(|e| e.net.is_some()), "{text}");
    }

    #[test]
    fn the_numbers_are_the_studio_starter_numbers() {
        // Same YAML node as `presets::starter`, apart from the flavor label and the spec.
        let text = render_config(&params(Style::EventLoop), &[]);
        let doc: serde_yaml_ng::Value = serde_yaml_ng::from_str(&text).unwrap();
        for (name, kind) in [("gateway", NodeKind::Gateway), ("app", NodeKind::Service)] {
            let mut mine = doc["nodes"][name].clone();
            let map = mine.as_mapping_mut().unwrap();
            map.remove("flavor");
            map.remove("spec");
            let theirs: serde_yaml_ng::Value =
                serde_yaml_ng::from_str(presets::starter(kind)).unwrap();
            assert_eq!(mine, theirs, "{name}");
        }
    }

    #[test]
    fn thread_per_request_holds_across_calls_with_200_slots() {
        let text = render_config(&params(Style::ThreadPerRequest), &[]);
        assert!(text.contains("hold: call"), "{text}");
        assert!(text.contains("concurrency: { limit: 200, queue: 100 }, proc: 25ms"));
        assert!(Config::from_yaml(&text).is_ok());
        assert!(!text.contains("limit: 32"), "{text}");
    }

    #[test]
    fn a_spec_path_with_quotes_stays_one_string() {
        let mut p = params(Style::EventLoop);
        p.spec = "a \"b\"\\c.yaml".into();
        let cfg = Config::from_yaml(&render_config(&p, &[])).unwrap();
        let NodeConfig::Service(s) = &cfg.nodes["app"] else {
            panic!()
        };
        assert_eq!(s.spec.as_deref(), Some("a \"b\"\\c.yaml"));
    }

    #[test]
    fn endpoints_are_chosen_by_method_and_path() {
        let routes = keys(&[
            "GET /users",
            "POST /users",
            "GET /users/{id}",
            "PUT /users/{id}",
            "PATCH /users/{id}",
            "DELETE /users/{id}",
            "GET /orders/{id}/items",
            "GET /things/{id}/",
            "GET /files/{*rest}",
            "HEAD /users",
            "OPTIONS /users",
        ]);
        let text = render_config(&params(Style::EventLoop), &routes);
        let cfg = Config::from_yaml(&text).unwrap();
        let Some(kaanal_core::config::EndpointsConfig::Inline(m)) = &cfg.endpoints else {
            panic!("no endpoints block: {text}")
        };
        let got: Vec<_> = m.keys().map(String::as_str).collect();
        // HEAD and OPTIONS get no entry
        assert_eq!(got.len(), 9, "{got:?}");
        assert!(text.contains("# endpoints:") && text.matches("estimate").count() >= 1);
        let line = |k: &str| {
            text.lines()
                .find(|l| l.trim_start().starts_with(&format!("\"{k}\":")))
                .unwrap_or_else(|| panic!("no line for {k}: {text}"))
        };
        let (read, scan, write) = (
            "op: read, chain: [{ cache: cache }, { call: database }]",
            "op: scan, chain: [{ call: database }]",
            "op: write, chain: [{ call: database }]",
        );
        for (k, want) in [
            ("GET /users", scan),
            ("GET /orders/{id}/items", scan),
            ("GET /users/{id}", read),
            ("GET /things/{id}/", read),
            ("GET /files/{*rest}", read),
            ("POST /users", write),
            ("PUT /users/{id}", write),
            ("PATCH /users/{id}", write),
            ("DELETE /users/{id}", write),
        ] {
            assert!(line(k).contains(want), "{k}: {}", line(k));
        }
        assert!(!text.contains("per_kb") && !text.contains("HEAD"));
    }

    #[test]
    fn a_spec_of_only_head_and_options_has_no_endpoints_block() {
        let text = render_config(
            &params(Style::EventLoop),
            &keys(&["HEAD /a", "OPTIONS /a/{id}"]),
        );
        assert!(!text.contains("endpoints:"), "{text}");
        assert!(Config::from_yaml(&text).is_ok());
    }

    #[test]
    fn the_peak_profile_has_a_journey_per_endpoint_and_at_most_64() {
        let text = render_config(
            &params(Style::EventLoop),
            &keys(&["GET /a", "POST /a", "HEAD /a", "DELETE /a/{id}"]),
        );
        let cfg = Config::from_yaml(&text).unwrap();
        let w: Vec<_> = cfg.traffic["peak"]
            .journeys
            .iter()
            .map(|j| (j.endpoint.as_str(), j.weight))
            .collect();
        assert_eq!(
            w,
            [("GET /a", 3.0), ("POST /a", 1.0), ("DELETE /a/{id}", 1.0)]
        );
        // more routes than a profile may hold: the file must still load
        let many: Vec<String> = (0..70).map(|i| format!("GET /r{i}")).collect();
        let cfg = Config::from_yaml(&render_config(&params(Style::EventLoop), &many)).unwrap();
        assert_eq!(cfg.traffic["peak"].journeys.len(), 64);
        let Some(kaanal_core::config::EndpointsConfig::Inline(m)) = &cfg.endpoints else {
            panic!("no endpoints")
        };
        assert_eq!(m.len(), 70);
    }

    #[test]
    fn the_summary_names_the_style_and_the_numbers() {
        assert_eq!(
            Style::EventLoop.summary(),
            "Chose: event loop, 32 slots, queue 100, 30 s timeout (estimates; edit kaanal.yaml)"
        );
        assert!(
            Style::ThreadPerRequest
                .summary()
                .starts_with("Chose: thread per request, 200 slots, queue 100, 30 s timeout")
        );
    }
}
