import { beforeEach, expect, test } from "vite-plus/test";
import type { LiveHello, Tick } from "../api/liveTypes";
import { MAX_WINDOW_MIN, MIN_WINDOW_MIN, useLive } from "./live";

const hello = (session: number, over: Partial<LiveHello> = {}): LiveHello => ({
  session,
  profile: "peak",
  note: null,
  rate: 100,
  paused: false,
  overloaded: false,
  in_flight: 0,
  submitted: 0,
  clients: 1,
  system_ms: 0,
  ...over,
});

const tick = (t_ms: number, over: Partial<Tick> = {}): Tick => ({
  t_ms,
  system_ms: t_ms,
  requests: 1,
  failed: 0,
  nodes: [],
  wires: [],
  hist: [],
  journeys: [],
  status: { rate: 100, paused: false, overloaded: false, in_flight: 0 },
  ...over,
});

beforeEach(() => {
  useLive.setState({ ...useLive.getInitialState(), windowMin: MIN_WINDOW_MIN });
  useLive.getState().reset();
});

test("the_ring_keeps_only_the_window", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  // ticks every 250 ms for 6 minutes: only the last 5 stay
  for (let t = 250; t <= 6 * 60_000; t += 250) s.ingest({ type: "tick", tick: tick(t) });
  const ticks = useLive.getState().ticks;
  expect(ticks.at(-1)?.t_ms).toBe(6 * 60_000);
  expect(ticks[0]?.t_ms).toBeGreaterThanOrEqual(6 * 60_000 - 5 * 60_000);
  expect(ticks.length).toBeLessThanOrEqual(5 * 60 * 4 + 1);
});

test("the_window_clamps_to_5_and_30_minutes", () => {
  const s = useLive.getState();
  s.setWindow(1);
  expect(useLive.getState().windowMin).toBe(MIN_WINDOW_MIN);
  s.setWindow(99);
  expect(useLive.getState().windowMin).toBe(MAX_WINDOW_MIN);
  s.setWindow(12.4);
  expect(useLive.getState().windowMin).toBe(12);
  s.setWindow(Number.NaN);
  expect(useLive.getState().windowMin).toBe(5);
});

test("a_longer_window_keeps_more_ticks", () => {
  const s = useLive.getState();
  s.setWindow(30);
  s.ingest({ type: "hello", hello: hello(1) });
  for (let t = 250; t <= 10 * 60_000; t += 250) s.ingest({ type: "tick", tick: tick(t) });
  expect(useLive.getState().ticks[0]?.t_ms).toBe(250);
});

test("a_reconnect_keeps_the_timeline_and_a_new_session_clears_it_with_a_marker", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  s.ingest({ type: "tick", tick: tick(250) });
  s.ingest({ type: "hello", hello: hello(1) });
  expect(useLive.getState().ticks).toHaveLength(1);
  s.ingest({ type: "hello", hello: hello(2) });
  const after = useLive.getState();
  expect(after.ticks).toHaveLength(0);
  expect(after.markers.map((m) => m.kind)).toEqual(["structure"]);
});

test("a_resync_leaves_a_gap_marker_and_keeps_the_ticks", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  s.ingest({ type: "tick", tick: tick(250) });
  s.ingest({ type: "resync", hello: hello(1) });
  const st = useLive.getState();
  expect(st.ticks).toHaveLength(1);
  expect(st.markers.at(-1)?.kind).toBe("gap");
});

test("old_ticks_drop_their_journeys_but_keep_their_counts", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  const journey = { hops: [], dropped_at: null, total_ns: 1 };
  s.ingest({ type: "tick", tick: tick(250, { journeys: [journey] }) });
  s.ingest({ type: "tick", tick: tick(30_000) });
  const [old, fresh] = useLive.getState().ticks;
  expect(old?.journeys).toEqual([]);
  expect(old?.requests).toBe(1);
  expect(fresh?.journeys).toEqual([]);
});

test("markers_pause_state_and_the_last_edit_are_recorded", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1, { paused: true }) });
  expect(useLive.getState().status).toBe("paused");
  s.ingest({ type: "marker", marker: { at_ms: 5, kind: "param", text: "app" } });
  expect(useLive.getState().markers).toHaveLength(1);
  s.noteEdit({ live: "applied", changed: ["app"], structure_pending: true });
  expect(useLive.getState().lastEdit).toEqual({
    live: "applied",
    changed: ["app"],
    structure_pending: true,
  });
});

// review finding 7
test("a_paused_generator_does_not_undo_the_hidden_pause", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  s.setStatus("hidden-paused");
  s.ingest({
    type: "tick",
    tick: tick(250, { status: { rate: 100, paused: true, overloaded: false, in_flight: 0 } }),
  });
  expect(useLive.getState().status).toBe("hidden-paused");
});

// review finding 9
test("a_restarted_session_clears_the_pending_structure_notice", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  s.noteEdit({ live: "unchanged", structure_pending: true });
  s.ingest({ type: "hello", hello: hello(2) });
  expect(useLive.getState().lastEdit).toBeNull();
});

// review finding 10: a tick costs work in proportion to what is new, and old journeys still go
test("ingest_drops_journeys_only_of_ticks_older_than_ten_seconds", () => {
  const s = useLive.getState();
  s.ingest({ type: "hello", hello: hello(1) });
  const j = [{ hops: [], dropped_at: null, total_ns: 1 }];
  for (let t = 250; t <= 20_000; t += 250)
    s.ingest({ type: "tick", tick: tick(t, { journeys: j }) });
  const ticks = useLive.getState().ticks;
  expect(ticks.find((t) => t.t_ms === 5_000)?.journeys).toEqual([]);
  expect(ticks.find((t) => t.t_ms === 19_000)?.journeys).toEqual(j);
  expect(ticks.at(-1)?.journeys).toEqual(j);
});
