//! The tests of this module, kept in their own file so the module stays under the size limit.

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
    // the routes of the offered mix, which a sampled journey's `stream` indexes
    assert_eq!(
        st.live.status().unwrap()["streams"],
        serde_json::json!(["GET /users/{id}"])
    );
    let mut rx = st.live.subscribe().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    while let Ok(ev) = rx.try_recv() {
        if let LiveEvent::Tick(t) = &*ev {
            assert_eq!(t.nodes.len(), 3);
        }
    }
}

/// Step 8A: a request the mock served is in the next tick, counted in the total and in the
/// real share, and drawn as a real journey.
#[tokio::test(start_paused = true)]
async fn a_mock_request_shows_in_the_next_tick_as_real() {
    let st = state(YAML);
    let _c = st.live.attach(&st, true);
    st.live.pause().unwrap(); // no generated requests: every figure below is the real one
    let mut rx = st.live.subscribe().unwrap();
    let req = axum::http::Request::builder()
        .method("GET")
        .uri("/users/1")
        .body(axum::body::Body::empty())
        .unwrap();
    let r = crate::mock::handle(axum::extract::State(Arc::clone(&st)), req).await;
    assert_eq!(r.status(), 200);
    tokio::time::sleep(Duration::from_millis(600)).await;
    let (mut total, mut real, mut journeys) = (0, 0, Vec::new());
    while let Ok(ev) = rx.try_recv() {
        if let LiveEvent::Tick(t) = &*ev {
            total += t.requests;
            real += t.real.as_ref().map_or(0, |r| r.requests);
            journeys.extend(t.journeys.iter().filter(|j| j.real).cloned());
        }
    }
    assert_eq!((total, real), (1, 1));
    assert_eq!(journeys.len(), 1);
}
