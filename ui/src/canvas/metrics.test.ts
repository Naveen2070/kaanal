import { expect, test } from "vite-plus/test";
import { nodeSummary } from "../test/summary";
import { capacityLine, nodeFigures, percent, twoSignificant } from "./metrics";

test("figures_take_the_peak_from_the_busy_series", () => {
  const f = nodeFigures(nodeSummary(), { type: "service" });
  expect(f.peak).toBe(0.9);
  expect(f.busy).toBe(0.5);
  expect(f.queue).toEqual([0, 3, 1]);
});

test("headroom_text_rounds_to_two_significant_digits", () => {
  const text = (n: number) =>
    nodeFigures(nodeSummary({ saturates_at_req_s: n }), { type: "service" }).headroomText;
  expect(text(122.4)).toBe("saturates near 120 req/s");
  expect(text(1530)).toBe("saturates near 1,500 req/s");
  expect(text(7.46)).toBe("saturates near 7.5 req/s");
  expect(twoSignificant(0.043)).toBe("0.043");
});

test("a_node_with_no_slots_shows_no_bar_and_no_headroom", () => {
  const f = nodeFigures(
    nodeSummary({
      utilization: null,
      offered_load: null,
      capacity_per_s: null,
      saturates_at_req_s: null,
      headroom: null,
      series: { window_s: 1, busy: [], queue: [0, 0], latency: [] },
    }),
    { type: "gateway" },
  );
  expect(f.busy).toBeNull();
  expect(f.peak).toBeNull();
  expect(f.headroomText).toBeNull();
});

test("a_node_with_no_run_has_no_figures_at_all", () => {
  const f = nodeFigures(undefined, { type: "service", replicas: 3 });
  expect(f).toMatchObject({ busy: null, peak: null, waiting: null, hasRun: false, replicas: 3 });
  expect(percent(f.busy)).toBe("–");
});

test("replicas_default_to_one_and_ignore_nonsense", () => {
  for (const r of [undefined, 0, -2, 1.5, "3", null]) {
    expect(nodeFigures(undefined, { type: "service", replicas: r }).replicas).toBe(1);
  }
});

// ---- T2.1: one source for the figures a card, the sentence and the KPI strip all show ----

import type { Summary } from "../api/types";
import { breaksFirst } from "./metrics";

const sentence = (over = {}, config = { type: "service" }) =>
  nodeFigures(nodeSummary(over), config).sentence;

test("sentence_says_up_to_for_peak_waiting", () => {
  expect(sentence({ utilization: 0.78, max_queue: 12 })).toBe("78% busy · up to 12 waiting");
  expect(sentence({ utilization: 0.5, max_queue: 1234 })).toBe("50% busy · up to 1,234 waiting");
  expect(sentence({ utilization: 0.5, max_queue: 0 })).toBe("50% busy · nothing waiting");
});

test("sentence_for_an_idle_node_never_says_nan", () => {
  expect(sentence({ utilization: 0, visits_per_s: 0, max_queue: 0 })).toBe("Idle");
  const worst = nodeFigures(
    nodeSummary({ utilization: null, visits_per_s: Number.NaN, p99_ms: null }),
    { type: "gateway" },
  );
  expect(worst.sentence).not.toMatch(/NaN|undefined|null/);
  expect(nodeFigures(undefined, { type: "service" }).sentence).toBe("No run yet");
});

test("a_saturated_node_says_so_in_words_not_just_colour", () => {
  expect(sentence({ saturated: true, utilization: 1, max_queue: 40 })).toBe(
    "Saturated · up to 40 waiting",
  );
});

test("a_node_without_slots_says_not_set_and_never_saturates", () => {
  const s = sentence({ utilization: null, saturates_at_req_s: null }, { type: "gateway" });
  expect(s).toBe("No slot limit · never saturates");
});

test("the_card_extras_come_from_the_run", () => {
  const f = nodeFigures(nodeSummary({ visits_per_s: 1000, p99_ms: 120, max_queue: 7 }), {
    type: "service",
  });
  expect(f.peakWaiting).toBe(7);
  expect(f.arrivingPerS).toBe(1000);
  expect(f.slowestUnderMs).toBe(120);
  const none = nodeFigures(undefined, { type: "service" });
  expect([none.peakWaiting, none.arrivingPerS, none.slowestUnderMs]).toEqual([0, null, null]);
});

function summaryOf(...nodes: ReturnType<typeof nodeSummary>[]): Summary {
  return { nodes } as unknown as Summary;
}

test("breaksFirst_picks_the_lowest_saturation_point", () => {
  const s = summaryOf(
    nodeSummary({ name: "web", saturates_at_req_s: 900 }),
    nodeSummary({ name: "db", saturates_at_req_s: 120, saturated: true }),
    nodeSummary({ name: "cache", saturates_at_req_s: 5000 }),
  );
  expect(breaksFirst(s)).toEqual({ node: "db", atReqS: 120, saturatedNow: true });
  const calm = summaryOf(nodeSummary({ name: "web", saturates_at_req_s: 900, saturated: false }));
  expect(breaksFirst(calm)).toEqual({ node: "web", atReqS: 900, saturatedNow: false });
});

test("breaksFirst_null_without_slots_or_visits", () => {
  expect(breaksFirst(undefined)).toBeNull();
  expect(breaksFirst(summaryOf())).toBeNull();
  const noCap = summaryOf(
    nodeSummary({ saturates_at_req_s: null }),
    nodeSummary({ saturates_at_req_s: 0 }),
    nodeSummary({ saturates_at_req_s: Number.POSITIVE_INFINITY }),
  );
  expect(breaksFirst(noCap)).toBeNull();
});

test("figures_agree_between_card_sentence_and_kpi", () => {
  const s = summaryOf(
    nodeSummary({ name: "web", saturates_at_req_s: 900 }),
    nodeSummary({ name: "db", saturates_at_req_s: 122.4, saturated: true, utilization: 1 }),
  );
  const kpi = breaksFirst(s)!;
  const card = nodeFigures(
    s.nodes.find((n) => n.name === kpi.node),
    { type: "database" },
  );
  // the headroom line on the card and the KPI's figure are one number, formatted once
  expect(card.headroomText).toBe(`saturates near ${twoSignificant(kpi.atReqS)} req/s`);
  expect(card.saturated).toBe(kpi.saturatedNow);
  expect(card.sentence.startsWith("Saturated")).toBe(kpi.saturatedNow);
});

test("capacity_line_reads_slots_copies_and_time_in_words", () => {
  expect(capacityLine(32, 25, 1)).toBe("Can finish up to 1,280/s · 32 at once ÷ 25 ms");
  expect(capacityLine(32, 25, 3)).toBe("Can finish up to 3,840/s · 3 × 32 at once ÷ 25 ms");
  expect(capacityLine(1, 60_000, 1)).toBe("Can finish up to 0.017/s · 1 at once ÷ 60,000 ms");
  expect(capacityLine(1, 400, 1)).toBe("Can finish up to 2.5/s · 1 at once ÷ 400 ms");
  expect(capacityLine(4, 0, 1)).toBe("No limit · 4 at once ÷ 0 ms"); // never Infinity or NaN
  expect(capacityLine(null, 25, 1)).toBe("No slot limit · never saturates");
  expect(capacityLine(32, null, 1)).toBeNull();
});
