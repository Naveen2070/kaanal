import { expect, test } from "vite-plus/test";
import { runLine, systemCounts } from "./counts";

test("parts_are_counted_by_sort_and_wires_by_edge", () => {
  const c = systemCounts({
    nodes: {
      gw: { type: "gateway" },
      a: { type: "service" },
      m: { type: "monolith" },
      db: { type: "database" },
      c: { type: "cache" },
      x: { type: "external-api" },
    },
    edges: [{ from: "gw", to: "a" }],
  });
  expect(c).toEqual({
    components: 6,
    traffic: 1,
    compute: 2,
    data: 2,
    external: 1,
    connections: 1,
  });
});

test("no_draft_counts_nothing", () => {
  expect(systemCounts(null).components).toBe(0);
  expect(systemCounts({}).connections).toBe(0);
});

test("the_status_line_says_how_long_the_last_run_took_and_when_one_is_running", () => {
  const took = (ms: number) => ({ started_ms: 1000, finished_ms: 1000 + ms });
  expect(runLine("done", took(400))).toBe("re-run in 0.4 s");
  expect(runLine("done", took(1234))).toBe("re-run in 1.2 s");
  expect(runLine("done", took(12_345))).toBe("re-run in 12 s");
  expect(runLine("running", took(400))).toBe("re-running..."); // the old time is not shown while a new run is going
  expect(runLine("running", null)).toBe("re-running...");
  // before the first run, or a run still going in the record, there is nothing to say
  expect(runLine("idle", null)).toBeNull();
  expect(runLine("done", { started_ms: 1, finished_ms: null })).toBeNull();
  // a blocked run says nothing here: its reason is a notice of its own
  expect(runLine("blocked", null)).toBeNull();
});
