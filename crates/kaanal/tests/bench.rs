//! Workspace benchmark suite: core, sim, data, server and overall (end-to-end HTTP).
//!
//! Run (release, ~30 s):
//! `cargo test -p kaanal --release --test bench -- --ignored --nocapture`
//!
//! Prints a table and writes JSON to `target/bench.json` (or `$KAANAL_BENCH_OUT`).
//! `KAANAL_BENCH_COMPARE=<file>` adds a column with the % change of each median against a
//! saved run. Keep baselines local and untracked. Nothing here fails on a regression: the only
//! hard check is NFR-05 (each 1M-request simulator run under 10 s with more than 990k requests).
//!
//! Every row runs 5 times and reports the median, min and max. The server and overall rows go
//! over loopback HTTP and are noisy: compare them only on the same machine, ideally idle.
//! Both serve a zero-latency topology, so they measure Kaanal's own overhead, not configured
//! hop times.

#![expect(
    clippy::cast_precision_loss,
    reason = "timings and counts become f64 for reporting only"
)]

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::exec::{ChainRequest, execute, sample_plan};
use kaanal_core::{ChaosState, Config, Topology};
use kaanal_data::{DEFAULT_LOCALE, DataSet, Dictionary, Options, Pack, builtin_tags, generate};
use kaanal_server::ServeOpts;
use kaanal_sim::{SimConfig, Stream, run, summarize};
use serde_json::{Value, json};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const REPS: usize = 5;
/// The overall rows: concurrent HTTP clients, requests per client per rep.
const CLIENTS: u64 = 8;
const EACH: u64 = 250;

/// The topology of the original NFR-05 bench (kept so its numbers stay comparable).
const SIM_YAML: &str = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 1ms, p99: 3ms } }\n  app: { type: service, concurrency: 4, proc: { mean: 2ms } }\n  db: { type: database, concurrency: 2, ops: { read: { mean: 0.5ms }, write: 1ms, scan: 3ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }\n";

/// Zero-duration hops over the server fixture's `openapi.yaml`: HTTP rows measure Kaanal, not sleeps.
const SERVE_YAML: &str = "version: 1\nseed: 42\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, ops: { read: 0ms, write: 0ms, scan: 0ms } }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n";

struct Row {
    name: String,
    unit: &'static str,
    median: f64,
    min: f64,
    max: f64,
}

fn row(name: &str, unit: &'static str, mut v: Vec<f64>) -> Row {
    v.sort_by(f64::total_cmp);
    Row {
        name: name.into(),
        unit,
        median: v[v.len() / 2],
        min: v[0],
        max: v[v.len() - 1],
    }
}

/// `f` called `ops` times per rep (after a 10 % warm-up); one sample = ns per call.
fn per_op(ops: u64, mut f: impl FnMut(u64)) -> Vec<f64> {
    (0..ops / 10).for_each(&mut f);
    (0..REPS)
        .map(|_| {
            let t = Instant::now();
            (0..ops).for_each(&mut f);
            t.elapsed().as_nanos() as f64 / ops as f64
        })
        .collect()
}

#[test]
#[ignore = "benchmark suite; run in release with --ignored --nocapture"]
fn suite() {
    let mut rows = Vec::new();
    core(&mut rows);
    sim(&mut rows);
    data(&mut rows);
    http(&mut rows);
    report(&rows);
}

fn core(rows: &mut Vec<Row>) {
    let cfg = Config::from_yaml(SIM_YAML).unwrap();
    rows.push(row(
        "core/topology_build",
        "ns/op",
        per_op(5_000, |_| {
            black_box(Topology::build(&cfg).unwrap());
        }),
    ));
    let t = Topology::build(&cfg).unwrap();
    let spec = RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap());
    rows.push(row(
        "core/chain_compile",
        "ns/op",
        per_op(5_000, |_| {
            black_box(compile(&t, std::slice::from_ref(&spec), EndpointsInput::default()).unwrap());
        }),
    ));
    let eps = compile(&t, &[spec], EndpointsInput::default()).unwrap();
    let endpoint = eps.get("GET /users/{id}").unwrap();
    let chaos = ChaosState::default().compile();
    let req = |n| ChainRequest {
        endpoint,
        n,
        seed: 1,
        pin_delay: Duration::ZERO,
        req_bytes: 0,
        resp_bytes: 0,
    };
    rows.push(row(
        "core/execute",
        "ns/op",
        per_op(200_000, |n| {
            black_box(execute(&t, &chaos, &req(n)));
        }),
    ));
    rows.push(row(
        "core/sample_plan",
        "ns/op",
        per_op(200_000, |n| {
            black_box(sample_plan(&t, &chaos, &req(n)));
        }),
    ));
}

/// A rated variant of [`SIM_YAML`]: a 25 MB/s wire into `app`, a 25 MB/s card on `app` and 12 kB of
/// body per request at 1000 a second (about half the wire, so several transfers share it at once
/// and the fair-sharing solver (RULES 62) runs on most events).
const SIM_RATED_YAML: &str = "version: 1
nodes:
  gw: { type: gateway, proc: { p50: 1ms, p99: 3ms } }
  app: { type: service, concurrency: 4, proc: { mean: 2ms }, bandwidth: 25000000 }
  db: { type: database, concurrency: 2, ops: { read: { mean: 0.5ms }, write: 1ms, scan: 3ms } }
edges:
  - { from: gw, to: app, net: 1ms, bandwidth: 25000000 }
  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }
";

/// NFR-05, unchanged from the old `kaanal-sim` bench: 1M simulated requests through `run`, then the
/// same with rates and body sizes so the pipes are shared (`_rated` rows).
fn sim(rows: &mut Vec<Row>) {
    sim_case(rows, SIM_YAML, (0, 0), "");
    sim_case(rows, SIM_RATED_YAML, (2_000, 10_000), "_rated");
    // the same wire at about nine tenths full (22 kB a request): many transfers in flight at once,
    // the case that shows whether the solver's cost grows with the number of flows (RULES 63)
    sim_case(rows, SIM_RATED_YAML, (4_000, 18_000), "_rated_high");
}

fn sim_case(rows: &mut Vec<Row>, yaml: &str, (req_bytes, resp_bytes): (u64, u64), suffix: &str) {
    let t = Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap();
    let spec = RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap());
    let e = compile(&t, &[spec], EndpointsInput::default()).unwrap();
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /users/{id}".into(),
            rate_per_s: 1_000.0,
            req_bytes,
            resp_bytes,
        }],
        duration: Duration::from_secs(1_000),
        warmup: Duration::ZERO,
        seed: 1,
    };
    let (mut runs, mut sums) = (Vec::new(), Vec::new());
    for _ in 0..REPS {
        let start = Instant::now();
        let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
        let took = start.elapsed();
        let n = rep.requests.len();
        println!("NFR-05{suffix}: {n} simulated requests in {took:.2?}");
        // The suite's only hard checks: NFR-05 is about the original topology. The rated variant
        // pays for sharing the pipes (about 1 us a transfer, RULES 63) and a loaded machine swings
        // it past 10 s, so it asserts the count and reports a slow run instead of failing on it;
        // the baseline column is what guards its speed.
        assert!(n > 990_000, "NFR-05{suffix}: only {n} requests");
        if suffix.is_empty() {
            assert!(took < Duration::from_secs(10), "NFR-05: {took:?} >= 10 s");
        } else if took >= Duration::from_secs(10) {
            println!("NFR-05{suffix}: WARNING {took:?} >= 10 s (not a failure: see the baseline)");
        }
        runs.push(took.as_secs_f64() * 1e3);
        // the summary of the same run: percentiles, series, bottlenecks (what the Studio asks for)
        let start = Instant::now();
        black_box(summarize(&t, &e, &cfg, &rep));
        sums.push(start.elapsed().as_secs_f64() * 1e3);
    }
    rows.push(row(
        &format!("sim/one_million_requests{suffix}"),
        "ms",
        runs,
    ));
    rows.push(row(
        &format!("sim/summarize_one_million{suffix}"),
        "ms",
        sums,
    ));
}

fn data(rows: &mut Vec<Row>) {
    let user = json!({ "type": "object", "required": ["id", "firstName", "lastName", "email"], "properties": {
        "id": { "type": "integer", "minimum": 1 }, "firstName": { "type": "string" },
        "lastName": { "type": "string" }, "email": { "type": "string", "format": "email" },
        "phone": { "type": "string" }, "city": { "type": "string" }, "state": { "type": "string" },
        "pincode": { "type": "string" }, "createdAt": { "type": "string", "format": "date-time" },
        "status": { "type": "string", "enum": ["active", "suspended"] } } });
    let small = json!({ "type": "object", "properties": {
        "id": { "type": "integer" }, "name": { "type": "string" }, "active": { "type": "boolean" } } });
    let line = json!({ "type": "object", "properties": {
        "sku": { "type": "string" }, "qty": { "type": "integer", "minimum": 1 },
        "price": { "type": "number", "minimum": 1 } } });
    let order = json!({ "type": "object", "properties": {
        "id": { "type": "string", "format": "uuid" }, "user": user, "currency": { "type": "string" },
        "total": { "type": "number" }, "createdAt": { "type": "string", "format": "date-time" },
        "lines": { "type": "array", "minItems": 5, "maxItems": 5, "items": line } } });
    let large = json!({ "type": "array", "minItems": 100, "maxItems": 100, "items": order });
    let opts = Options::default();
    for (name, schema, ops) in [
        ("small", &small, 50_000),
        ("medium", &user, 20_000),
        ("large", &large, 200),
    ] {
        rows.push(row(
            &format!("data/generate_{name}"),
            "ns/op",
            per_op(ops, |i| {
                black_box(generate(schema, None, i, &opts));
            }),
        ));
    }
    let tag = builtin_tags()
        .iter()
        .find(|t| **t != DEFAULT_LOCALE)
        .expect("a second built-in pack");
    let locale = Options {
        pack: Pack::builtin(tag).unwrap(),
        ..Options::default()
    };
    rows.push(row(
        "data/generate_medium_locale",
        "ns/op",
        per_op(20_000, |i| {
            black_box(generate(&user, None, i, &locale));
        }),
    ));

    let dir = std::env::temp_dir().join(format!("kaanal-bench-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("locales")).unwrap();
    std::fs::write(
        dir.join("locales/xx-YY.yaml"),
        "version: 1\ntag: xx-YY\nname_order: given-family\ncountry: Nowhere\ncurrency: { code: TST, symbol: T }\nphone: { prefix: \"+0\", formats: [\"{4}\"] }\naddress: \"{number} {street}, {city}\"\nfirst_names: [Ann, Bo]\nlast_names: [Lee, Ng]\nstreets: [Main]\nplaces: [{ city: Town, postcode: \"{5}\" }]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("data.yaml"),
        "version: 1\nfields:\n  sku: { gen: template, value: \"SKU-{digits:6}\" }\n  \"*code\": { gen: template, value: \"{letters:3}\" }\n",
    )
    .unwrap();
    rows.push(row(
        "data/dataset_load",
        "ns/op",
        per_op(500, |_| {
            black_box(DataSet::load(&dir, None).unwrap());
        }),
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let dict = Dictionary::builtin();
    let fields = [
        "email",
        "firstName",
        "userId",
        "createdAt",
        "phoneNumber",
        "postcode",
        "zzUnknown",
        "orderTotal",
    ];
    rows.push(row(
        "data/dictionary_lookup",
        "ns/op",
        per_op(200_000, |i| {
            black_box(dict.contains(fields[(i % 8) as usize]));
        }),
    ));
}

/// The server in-process on an ephemeral port, as the server tests start it.
fn http(rows: &mut Vec<Row>) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir: PathBuf =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../kaanal-server/tests/fixtures");
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
        draft: None,
    };
    let srv = rt
        .block_on(kaanal_server::serve(
            &Config::from_yaml(SERVE_YAML).unwrap(),
            &dir,
            opts,
        ))
        .unwrap();
    let base = format!("http://{}", srv.addr);
    let agent = || -> ureq::Agent {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .build()
            .into()
    };
    let get = |a: &ureq::Agent, i: u64| {
        let mut r = a.get(&format!("{base}/users/{i}")).call().unwrap();
        black_box(r.body_mut().read_to_string().unwrap());
    };

    // server: one client, sequential, keep-alive; a generated `User` body per request. On an
    // idle runtime each request waits for the actor's timer (zero-duration hops still go through
    // `sleep_until`), so this row shows the timer floor (~15 ms on Windows), hence few ops.
    let a = agent();
    let rps = |v: Vec<f64>| v.into_iter().map(|ns| 1e9 / ns).collect();
    rows.push(row(
        "server/mock_get",
        "req/s",
        rps(per_op(100, |i| get(&a, i))),
    ));
    let auth = format!("Bearer {token}");
    let status = a
        .put(&format!("{base}/__kaanal/chaos/db"))
        .header("Authorization", &auth)
        .send_json(json!({ "error_rate": 0.5 }))
        .unwrap()
        .status();
    assert_eq!(status, 200, "setting chaos");
    rows.push(row(
        "server/mock_get_chaos50",
        "req/s",
        rps(per_op(100, |i| get(&a, i))),
    ));
    a.delete(&format!("{base}/__kaanal/chaos"))
        .header("Authorization", &auth)
        .call()
        .unwrap();

    // overall: CLIENTS concurrent clients, each its own connection.
    let (mut tput, mut p50, mut p95, mut p99) = (vec![], vec![], vec![], vec![]);
    for rep in 0..=REPS {
        let start = Instant::now();
        let mut lat: Vec<f64> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..CLIENTS)
                .map(|c| {
                    let get = &get;
                    s.spawn(move || {
                        let a = agent();
                        (0..EACH)
                            .map(|i| {
                                let t = Instant::now();
                                get(&a, c * EACH + i);
                                t.elapsed().as_secs_f64() * 1e6
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let took = start.elapsed().as_secs_f64();
        if rep == 0 {
            continue; // warm-up
        }
        lat.sort_by(f64::total_cmp);
        let pct = |p: usize| lat[(lat.len() - 1) * p / 100];
        tput.push(lat.len() as f64 / took);
        p50.push(pct(50));
        p95.push(pct(95));
        p99.push(pct(99));
    }
    rows.push(row("overall/http_8_clients", "req/s", tput));
    rows.push(row("overall/latency_p50", "us", p50));
    rows.push(row("overall/latency_p95", "us", p95));
    rows.push(row("overall/latency_p99", "us", p99));
    srv.shutdown(Duration::ZERO);
    rt.shutdown_background();
}

fn report(rows: &[Row]) {
    let base: Option<Value> = std::env::var("KAANAL_BENCH_COMPARE").ok().map(|p| {
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}")))
            .unwrap_or_else(|e| panic!("{p}: {e}"))
    });
    let old = |name: &str| {
        base.as_ref()?["rows"]
            .as_array()?
            .iter()
            .find(|r| r["name"] == name)?["median"]
            .as_f64()
    };
    println!(
        "\n{:<30} {:>12} {:>12} {:>12}  {:<6} {:>9}",
        "bench", "median", "min", "max", "unit", "vs base"
    );
    for r in rows {
        let change = match old(&r.name) {
            Some(o) if o != 0.0 => format!("{:+.1}%", (r.median - o) / o * 100.0),
            _ if base.is_some() => "new".into(),
            _ => String::new(),
        };
        println!(
            "{:<30} {:>12.1} {:>12.1} {:>12.1}  {:<6} {:>9}",
            r.name, r.median, r.min, r.max, r.unit, change
        );
    }
    let out = std::env::var_os("KAANAL_BENCH_OUT").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bench.json"),
        PathBuf::from,
    );
    let doc = json!({
        "reps": REPS,
        "rows": rows.iter().map(|r| json!({
            "name": r.name, "unit": r.unit, "median": r.median, "min": r.min, "max": r.max,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(&out, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
    println!("\nwrote {}", out.display());
}
