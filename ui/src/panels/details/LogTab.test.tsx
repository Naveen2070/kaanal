import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { LogLine, RunRecord } from "../../data/api/types";
import { useRun } from "../../data/store/run";
import { mockFetch } from "../../test/fetch";
import { LogTab } from "./LogTab";
import { formatLine } from "./logFormat";

function line(over: Partial<LogLine> = {}): LogLine {
  return {
    t_ms: 12310,
    level: "warn",
    request: 4812,
    stream: "POST /orders",
    node: "api",
    replica: 2,
    event: "queued",
    wait_ms: 210,
    service_ms: 98,
    status: null,
    note: null,
    ...over,
  };
}

function run(id = "s1"): RunRecord {
  return {
    id,
    state: "done",
    generation: 1,
    profile: "peak",
    scenario: null,
    source: "draft",
    draft_rev: 0,
    started_ms: 0,
    finished_ms: 1,
    summary: {
      label: "relative what-if",
      window_s: 10,
      requests: 10,
      failed: 0,
      errors: {},
      warnings: [],
      latency: { p50_ms: 1, p95_ms: 2, p99_ms: 3, max_ms: 4, buckets: [] },
      series: { window_s: 1, done: [], failed: [], p99_ms: [] },
      streams: [
        { endpoint: "POST /orders", requests: 5, failed: 0 },
        { endpoint: "GET /a b?c", requests: 5, failed: 0 },
      ],
      nodes: [],
      links: [],
      bottlenecks: [],
    },
  };
}

const page = (lines: LogLine[], next: number | null = null) => ({
  lines,
  next,
  total: lines.length,
  sampled: true,
  note: "A sample in virtual time, not every request.",
});

function mount() {
  history.replaceState(null, "", "/__kaanal/studio");
  render(
    <NuqsAdapter>
      <LogTab />
    </NuqsAdapter>,
  );
}

async function show(first = page([line()])) {
  useRun.setState({ status: "done", latest: run(), dimmed: false, lastError: null });
  const sent = mockFetch([{ json: first }]);
  mount();
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  return sent;
}

beforeEach(() => useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("formats_a_line_like_the_spec_example", () => {
  expect(formatLine(line())).toBe(
    "t=12.31s WARN r-4812 POST /orders api#2 queued 210ms, served 98ms",
  );
  expect(
    formatLine(
      line({
        level: "error",
        replica: null,
        event: "overflowed",
        wait_ms: null,
        service_ms: null,
        status: 503,
        note: "queue full",
      }),
    ),
  ).toBe("t=12.31s ERROR r-4812 POST /orders api overflowed 503 queue full");
  expect(formatLine(line({ level: "info", event: "served", note: "proc" }))).toBe(
    "t=12.31s INFO r-4812 POST /orders api#2 served 98ms (proc)",
  );
});

test("shows_the_sampled_note_and_says_the_time_is_virtual", async () => {
  await show();
  expect(await screen.findByText(/A sample in virtual time/)).toBeTruthy();
  expect(screen.getByText(/Run 1 · simulated, virtual time/)).toBeTruthy();
  expect(await screen.findByRole("button", { name: /t=12.31s WARN r-4812/ })).toBeTruthy();
});

test("before_any_run_it_says_so_and_asks_for_nothing", () => {
  const sent = mockFetch([{ json: page([]) }]);
  mount();
  expect(screen.getByText(/No finished run yet/)).toBeTruthy();
  expect(sent).toHaveLength(0);
});

test("filters_are_sent_as_query_parameters", async () => {
  await show();
  const sent = mockFetch([{ json: page([]) }]);
  fireEvent.click(screen.getByRole("button", { name: "Errors only" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?level=error&limit=200");
  fireEvent.change(screen.getByLabelText("Instance"), { target: { value: "2" } });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.url).toContain("replica=2");
  expect(sent[1]?.url).toContain("level=error");
});

test("names_with_special_characters_are_url_encoded_in_the_query", async () => {
  await show();
  const sent = mockFetch([{ json: page([]) }]);
  fireEvent.click(screen.getByRole("button", { name: "GET /a b?c" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?endpoint=GET+%2Fa+b%3Fc&limit=200");
});

test("paging_appends_and_stops_when_next_is_null", async () => {
  await show(page([line({ request: 1 })], 0));
  expect(await screen.findByText(/r-1 /)).toBeTruthy();
  const sent = mockFetch([{ json: page([line({ request: 2 })], null) }]);
  fireEvent.click(screen.getByRole("button", { name: /Load more/ }));
  await vi.waitFor(() => expect(screen.getByText(/r-2 /)).toBeTruthy());
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?limit=200&after=0");
  expect(screen.getByText(/r-1 /)).toBeTruthy();
  expect(screen.queryByRole("button", { name: /Load more/ })).toBeNull();
});

test("a_409_while_running_shows_waiting_not_an_error_and_a_404_says_it_is_gone", async () => {
  useRun.setState({ status: "done", latest: run(), dimmed: false, lastError: null });
  mockFetch([{ status: 409, json: { error: "the run is still running" } }]);
  mount();
  expect((await screen.findByRole("alert")).textContent).toMatch(/Waiting for the run/);
  cleanup();
  mockFetch([{ status: 404, json: { error: "no such run" } }]);
  mount();
  expect((await screen.findByRole("alert")).textContent).toMatch(/no longer kept/);
});

test("a_newer_run_replaces_the_log", async () => {
  await show(page([line({ request: 1 })]));
  expect(await screen.findByText(/r-1 /)).toBeTruthy();
  const sent = mockFetch([{ json: page([line({ request: 7 })]) }]);
  act(() => useRun.setState({ latest: run("s2") }));
  await vi.waitFor(() => expect(screen.getByText(/r-7 /)).toBeTruthy());
  expect(sent[0]?.url).toContain("/runs/s2/log");
  expect(screen.queryByText(/r-1 /)).toBeNull();
});

test("a_click_opens_that_request_hop_by_hop", async () => {
  await show(page([line({ request: 4812 })]));
  const sent = mockFetch([
    {
      json: page([
        line({
          request: 4812,
          node: "gw",
          replica: 0,
          event: "served",
          wait_ms: 0,
          service_ms: 2,
          t_ms: 100,
        }),
        line({ request: 4812, t_ms: 103 }),
      ]),
    },
  ]);
  fireEvent.click(await screen.findByRole("button", { name: /t=12.31s WARN r-4812/ }));
  await screen.findByRole("region", { name: "Request r-4812" });
  expect(sent[0]?.url).toBe(
    "/__kaanal/sim/runs/s1/log?endpoint=POST+%2Forders&request=4812&limit=1000",
  );
  expect(await screen.findByText("gw#0")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(screen.queryByRole("region", { name: "Request r-4812" })).toBeNull();
});

test("an_older_answer_arriving_late_never_replaces_the_newer_runs_log", async () => {
  useRun.setState({ status: "done", latest: run("s1"), dimmed: false, lastError: null });
  // the first answer is slow, the second quick: the mock cannot be aborted, so the guard must hold
  const sent = mockFetch([
    { json: page([line({ request: 1 })]), delayMs: 80 },
    { json: page([line({ request: 7 })]) },
  ]);
  mount();
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  act(() => useRun.setState({ latest: run("s2") }));
  await vi.waitFor(() => expect(screen.getByText(/r-7 /)).toBeTruthy());
  await new Promise((r) => setTimeout(r, 150));
  expect(screen.getByText(/r-7 /)).toBeTruthy();
  expect(screen.queryByText(/r-1 /)).toBeNull();
});

test("load_more_after_a_filter_change_does_not_append_lines_of_the_old_filter", async () => {
  await show(page([line({ request: 1 })], 0));
  await screen.findByText(/r-1 /);
  // a slow "load more" for the unfiltered key, then the filter changes under it
  mockFetch([
    { json: page([line({ request: 2 })]), delayMs: 60 },
    { json: page([line({ request: 9, level: "error" })]) },
  ]);
  fireEvent.click(screen.getByRole("button", { name: /Load more/ }));
  fireEvent.click(screen.getByRole("button", { name: "Errors only" }));
  await vi.waitFor(() => expect(screen.getByText(/r-9 /)).toBeTruthy());
  await new Promise((r) => setTimeout(r, 120));
  expect(screen.queryByText(/r-2 /)).toBeNull();
  expect(screen.getByText(/r-9 /)).toBeTruthy();
});

test("a_request_missing_from_a_newer_run_closes_the_waterfall", async () => {
  await show(page([line({ request: 4812 })]));
  mockFetch([{ json: page([line({ request: 4812 })]) }]);
  fireEvent.click(await screen.findByRole("button", { name: /r-4812/ }));
  await screen.findByRole("region", { name: "Request r-4812" });
  mockFetch([{ json: page([line({ request: 3 })]) }]);
  act(() => useRun.setState({ latest: run("s2") }));
  await vi.waitFor(() => expect(screen.getByText(/r-3 /)).toBeTruthy());
  expect(screen.queryByRole("region", { name: "Request r-4812" })).toBeNull();
});

test("levels_are_info_warn_error_with_text", async () => {
  await show(
    page([
      line({ level: "info", event: "served", request: 1 }),
      line({ level: "warn", request: 2 }),
      line({ level: "error", event: "overflowed", wait_ms: null, service_ms: null, request: 3 }),
    ]),
  );
  await screen.findByText(/r-1 /);
  for (const w of ["info", "warn", "error"]) expect(screen.getByText(w)).toBeTruthy();
});

test("event_cell_is_a_button_inside_a_table_with_a_time_column", async () => {
  await show();
  const b = await screen.findByRole("button", { name: /r-4812/ });
  expect(b.closest("td")).toBeTruthy();
  expect(screen.getByText("00:12.310")).toBeTruthy();
  expect(screen.getByRole("columnheader", { name: "ms" })).toBeTruthy();
});

test("header_says_simulated_virtual_time_and_sample_count", async () => {
  await show(page([line()]));
  expect(await screen.findByText(/1 of 1 hops sampled/)).toBeTruthy();
});

test("a_filter_the_run_lacks_is_dropped_and_says_so", async () => {
  await show();
  fireEvent.click(screen.getByRole("button", { name: "GET /a b?c" }));
  await vi.waitFor(() =>
    expect(screen.getByRole("button", { name: "GET /a b?c" }).getAttribute("aria-pressed")).toBe(
      "true",
    ),
  );
  mockFetch([{ json: page([line()]) }]);
  act(() => {
    const r = run("s2");
    if (r.summary) r.summary.streams = [{ endpoint: "POST /orders", requests: 1, failed: 0 }];
    useRun.setState({ latest: r });
  });
  expect(
    await screen.findByText(/no endpoint .GET \/a b\?c., so that filter was dropped/),
  ).toBeTruthy();
});

test("empty_and_error_states_stay_in_the_frame_and_error_has_retry", async () => {
  useRun.setState({ status: "done", latest: run(), dimmed: false, lastError: null });
  mockFetch([{ json: page([]) }]);
  mount();
  const empty = await screen.findByText("No lines match these filters.");
  expect(empty.parentElement?.className).toMatch(/min-h-40/);
  cleanup();
  const sent = mockFetch([{ status: 500, json: { error: "boom" } }, { json: page([line()]) }]);
  mount();
  fireEvent.click(await screen.findByRole("button", { name: "Retry" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(await screen.findByText(/r-4812 /)).toBeTruthy();
});

test("escape_closes_the_open_request", async () => {
  await show(page([line({ request: 4812 })]));
  mockFetch([{ json: page([line({ request: 4812 })]) }]);
  fireEvent.click(await screen.findByRole("button", { name: /r-4812/ }));
  await screen.findByRole("region", { name: "Request r-4812" });
  fireEvent.keyDown(window, { key: "Escape" });
  expect(screen.queryByRole("region", { name: "Request r-4812" })).toBeNull();
});

const open = async (hops: LogLine[]) => {
  await show(page([line({ request: 9 })]));
  mockFetch([{ json: page(hops) }]);
  fireEvent.click(await screen.findByRole("button", { name: /r-9/ }));
  await screen.findByRole("region", { name: "Request r-9" });
};

test("waterfall_wait_finding_only_when_wait_ms_present", async () => {
  await open([
    line({ request: 9, node: "service", replica: 0, t_ms: 0, wait_ms: 38, service_ms: 8 }),
  ]);
  expect(
    await screen.findByText("Most of it, 38 ms, was spent waiting in line at service"),
  ).toBeTruthy();
});

test("failed_request_shows_hops_and_failure_without_wait_split", async () => {
  await open([
    line({
      request: 9,
      node: "gw",
      replica: 0,
      t_ms: 0,
      event: "served",
      wait_ms: 0,
      service_ms: 2,
    }),
    line({
      request: 9,
      node: "db",
      replica: null,
      t_ms: 5,
      level: "error",
      event: "overflowed",
      wait_ms: null,
      service_ms: null,
      status: 503,
      note: "queue full",
    }),
  ]);
  expect(await screen.findByText("gw#0")).toBeTruthy();
  expect(screen.getByText("Failed 503 queue full")).toBeTruthy();
  expect(screen.getByText("Little of it was spent waiting in line")).toBeTruthy();
});

test("a_request_with_no_wait_ms_at_all_has_no_finding", async () => {
  await open([
    line({
      request: 9,
      node: "db",
      replica: null,
      level: "error",
      event: "timeout",
      wait_ms: null,
      service_ms: null,
      status: 504,
      note: "deadline passed",
    }),
  ]);
  await screen.findByText("Failed 504 deadline passed");
  expect(screen.queryByText(/waiting in line/)).toBeNull();
});
