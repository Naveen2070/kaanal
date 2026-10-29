import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { LogLine, RunRecord } from "../api/types";
import { useRun } from "../store/run";
import { mockFetch } from "../test/fetch";
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
  expect(screen.getByText(/Virtual time inside the simulation/)).toBeTruthy();
  expect(await screen.findByText(/t=12.31s WARN r-4812/)).toBeTruthy();
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
  fireEvent.click(screen.getByLabelText("Errors only"));
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
  fireEvent.change(screen.getByLabelText("Endpoint"), { target: { value: "GET /a b?c" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?endpoint=GET+%2Fa+b%3Fc&limit=200");
});

test("paging_appends_and_stops_when_next_is_null", async () => {
  await show(page([line({ request: 1 })], 0));
  expect(await screen.findByText(/r-1 /)).toBeTruthy();
  const sent = mockFetch([{ json: page([line({ request: 2 })], null) }]);
  fireEvent.click(screen.getByRole("button", { name: "Load more" }));
  await vi.waitFor(() => expect(screen.getByText(/r-2 /)).toBeTruthy());
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?limit=200&after=0");
  expect(screen.getByText(/r-1 /)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
});

test("a_409_while_running_shows_waiting_not_an_error_and_a_404_says_it_is_gone", async () => {
  useRun.setState({ status: "done", latest: run(), dimmed: false, lastError: null });
  mockFetch([{ status: 409, json: { error: "the run is still running" } }]);
  mount();
  expect((await screen.findByRole("alert")).textContent).toMatch(/still going/);
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
  fireEvent.click(screen.getByRole("button", { name: "Load more" }));
  fireEvent.click(screen.getByLabelText("Errors only"));
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
