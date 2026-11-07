import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, LogLine, RunRecord } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { expectNoViolations } from "../../test/axe";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { LogTab } from "./LogTab";
import { useLogDrawer } from "./logDrawer";
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
  return render(
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

const filters = () => fireEvent.click(screen.getByRole("button", { name: /^Filters/ }));
const header = (n: number) => screen.getByRole("button", { name: new RegExp(`^r-${n}\\b`) });

beforeEach(() => {
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
  useDraft.setState({ view: null });
  useLogDrawer.getState().set(false);
});
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
  expect(screen.getByText(/Simulated trace events, not process output/)).toBeTruthy();
  expect(await screen.findByText("api#2")).toBeTruthy();
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
  filters();
  fireEvent.change(screen.getByLabelText("Instance"), { target: { value: "2" } });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.url).toContain("replica=2");
  expect(sent[1]?.url).toContain("level=error");
  // the button counts what is on inside it, and Clear puts everything back
  expect(screen.getByRole("button", { name: "Filters (1 on)" })).toBeTruthy();
  mockFetch([{ json: page([]) }]);
  fireEvent.click(screen.getByRole("button", { name: "Clear" }));
  expect(screen.getByRole("button", { name: "All" }).getAttribute("aria-pressed")).toBe("true");
  expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
});

test("one_component_or_one_wire_is_a_filter_and_the_two_do_not_mix", async () => {
  const r = run();
  const link = {
    from: "gw",
    to: "api",
    transfers: 4,
    mean_wait_ms: 1,
    offered_load: 0.5,
    utilization: 0.5,
    saturated: false,
    saturates_at_req_s: 40,
    series: { window_s: 1, latency: [] },
  };
  if (r.summary) {
    r.summary.nodes = [nodeSummary({ name: "gw" }), nodeSummary({ name: "api" })];
    r.summary.links = [link];
  }
  useRun.setState({ status: "done", latest: r, dimmed: false, lastError: null });
  const sent = mockFetch([{ json: page([]) }]);
  mount();
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  filters();
  fireEvent.change(screen.getByLabelText("Component"), { target: { value: "api" } });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.url).toBe("/__kaanal/sim/runs/s1/log?node=api&limit=200");
  // picking the wire drops the component: a wire is two components
  fireEvent.change(screen.getByLabelText("Wire"), { target: { value: "gw\u001fapi" } });
  await vi.waitFor(() => expect(sent).toHaveLength(3));
  expect(sent[2]?.url).toBe("/__kaanal/sim/runs/s1/log?from=gw&to=api&limit=200");
  expect((screen.getByLabelText("Component") as HTMLSelectElement).value).toBe("");
});

test("a_run_with_no_bandwidth_wire_disables_the_wire_filter_and_says_why", async () => {
  await show();
  filters();
  const wire = screen.getByLabelText("Wire") as HTMLSelectElement;
  expect(wire.disabled).toBe(true);
  expect(wire.title).toMatch(/bandwidth/);
});

test("names_with_special_characters_are_url_encoded_in_the_query", async () => {
  await show();
  filters();
  const sent = mockFetch([{ json: page([]) }]);
  fireEvent.click(screen.getByRole("button", { name: "GET /a b?c" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?endpoint=GET+%2Fa+b%3Fc&limit=200");
});

test("more_than_three_endpoints_become_a_select", async () => {
  const r = run();
  if (r.summary) {
    r.summary.streams = ["a", "b", "c", "d"].map((e) => ({ endpoint: e, requests: 1, failed: 0 }));
  }
  useRun.setState({ status: "done", latest: r, dimmed: false, lastError: null });
  const sent = mockFetch([{ json: page([]) }]);
  mount();
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  filters();
  expect(screen.queryByRole("button", { name: "c" })).toBeNull();
  fireEvent.change(screen.getByLabelText("Endpoint"), { target: { value: "c" } });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.url).toBe("/__kaanal/sim/runs/s1/log?endpoint=c&limit=200");
});

test("paging_appends_and_a_request_cut_by_the_page_edge_is_one_block", async () => {
  await show(
    page([line({ request: 1, node: "gw", t_ms: 0 }), line({ request: 2, node: "gw", t_ms: 1 })], 1),
  );
  await screen.findByText("r-1");
  const sent = mockFetch([{ json: page([line({ request: 1, node: "db", t_ms: 5 })], null) }]);
  fireEvent.click(screen.getByRole("button", { name: /Load more/ }));
  await vi.waitFor(() => expect(screen.getByText("db#2")).toBeTruthy());
  expect(sent[0]?.url).toBe("/__kaanal/sim/runs/s1/log?limit=200&after=1");
  // r-1 is one block holding both of its hops, and r-2 is the other
  expect(screen.getAllByText("r-1")).toHaveLength(1);
  const block = header(1).closest("li") as HTMLElement;
  expect(within(block).getByText("gw#2")).toBeTruthy();
  expect(within(block).getByText("db#2")).toBeTruthy();
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
  expect(await screen.findByText("r-1")).toBeTruthy();
  const sent = mockFetch([{ json: page([line({ request: 7 })]) }]);
  act(() => useRun.setState({ latest: run("s2") }));
  await vi.waitFor(() => expect(screen.getByText("r-7")).toBeTruthy());
  expect(sent[0]?.url).toContain("/runs/s2/log");
  expect(screen.queryByText("r-1")).toBeNull();
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
  await vi.waitFor(() => expect(screen.getByText("r-7")).toBeTruthy());
  await new Promise((r) => setTimeout(r, 150));
  expect(screen.getByText("r-7")).toBeTruthy();
  expect(screen.queryByText("r-1")).toBeNull();
});

test("load_more_after_a_filter_change_does_not_append_lines_of_the_old_filter", async () => {
  await show(page([line({ request: 1 })], 0));
  await screen.findByText("r-1");
  // a slow "load more" for the unfiltered key, then the filter changes under it
  mockFetch([
    { json: page([line({ request: 2 })]), delayMs: 60 },
    { json: page([line({ request: 9, level: "error" })]) },
  ]);
  fireEvent.click(screen.getByRole("button", { name: /Load more/ }));
  fireEvent.click(screen.getByRole("button", { name: "Errors only" }));
  await vi.waitFor(() => expect(screen.getByText("r-9")).toBeTruthy());
  await new Promise((r) => setTimeout(r, 120));
  expect(screen.queryByText("r-2")).toBeNull();
  expect(screen.getByText("r-9")).toBeTruthy();
});

test("every_hop_carries_a_type_badge_and_a_failure_its_cause", async () => {
  await show(
    page([
      line({ request: 1, event: "served", wait_ms: 0, note: "proc" }),
      line({ request: 1, t_ms: 12320, node: "db", event: "served", note: "op" }),
      line({ request: 1, t_ms: 12330, node: "cache", event: "served", note: "cache hit" }),
      line({
        request: 2,
        level: "error",
        event: "overflowed",
        wait_ms: null,
        service_ms: null,
        status: 503,
        note: "queue full",
      }),
    ]),
  );
  await screen.findByText("r-1");
  for (const w of ["Work", "Data", "Cache hit", "Queue full"]) {
    expect(screen.getAllByText(w).length).toBeGreaterThan(0);
  }
  // one status mark per request: ok for the served one, the failure word for the other
  expect(within(header(1)).getByText("ok")).toBeTruthy();
  expect(within(header(2)).getByText("Queue full")).toBeTruthy();
});

test("a_request_block_opens_and_closes_from_its_header_and_the_first_few_start_open", async () => {
  const lines = [1, 2, 3, 4, 5, 6].map((n) => line({ request: n, node: `n${n}` }));
  await show(page(lines));
  await screen.findByText("r-1");
  expect(header(1).getAttribute("aria-expanded")).toBe("true");
  expect(header(5).getAttribute("aria-expanded")).toBe("true");
  expect(header(6).getAttribute("aria-expanded")).toBe("false");
  expect(screen.queryByText("n6#2")).toBeNull();
  fireEvent.click(header(6));
  expect(header(6).getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByText("n6#2")).toBeTruthy();
  fireEvent.click(header(1));
  expect(screen.queryByText("n1#2")).toBeNull();
});

test("escape_collapses_the_request_in_focus_and_closes_the_filters_first", async () => {
  await show(page([line({ request: 4812 })]));
  await screen.findByText("r-4812");
  filters();
  expect(screen.getByLabelText("Instance")).toBeTruthy();
  fireEvent.keyDown(header(4812), { key: "Escape" });
  expect(screen.queryByLabelText("Instance")).toBeNull();
  expect(header(4812).getAttribute("aria-expanded")).toBe("true");
  fireEvent.keyDown(header(4812), { key: "Escape" });
  expect(header(4812).getAttribute("aria-expanded")).toBe("false");
});

test("hops_are_indented_by_their_components_depth_in_the_call_graph", async () => {
  useDraft.setState({
    view: {
      doc: {
        nodes: {},
        edges: [
          { from: "gw", to: "api" },
          { from: "api", to: "db" },
        ],
      },
    } as unknown as DraftView,
  });
  await show(
    page([
      line({ request: 1, node: "gw", t_ms: 0 }),
      line({ request: 1, node: "api", t_ms: 1 }),
      line({ request: 1, node: "db", t_ms: 2 }),
    ]),
  );
  await screen.findByText("gw#2");
  const pad = (name: string) => (screen.getByText(name) as HTMLElement).style.paddingLeft;
  expect([pad("gw#2"), pad("api#2"), pad("db#2")]).toEqual(["0px", "12px", "24px"]);
});

test("no_graph_means_every_hop_at_depth_zero", async () => {
  await show(page([line({ request: 1, node: "api" })]));
  expect((await screen.findByText("api#2")).style.paddingLeft).toBe("0px");
});

test("the_top_line_is_the_one_finding_from_the_loaded_sample", async () => {
  await show(page([line({ request: 1, wait_ms: 9300, node: "api" })]));
  expect(await screen.findByText("api is where requests waited: 9.3 s at worst")).toBeTruthy();
  expect(screen.getByText("Waited 9.3 s at api")).toBeTruthy();
});

test("header_says_simulated_virtual_time_and_sample_count", async () => {
  await show(page([line()]));
  expect(await screen.findByText(/1 of 1 hops sampled/)).toBeTruthy();
});

test("a_filter_the_run_lacks_is_dropped_and_says_so", async () => {
  await show();
  filters();
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
  const empty = await screen.findByText(/This run kept no requests/);
  expect(empty.parentElement?.className).toMatch(/min-h-40/);
  cleanup();
  const sent = mockFetch([{ status: 500, json: { error: "boom" } }, { json: page([line()]) }]);
  mount();
  fireEvent.click(await screen.findByRole("button", { name: "Retry" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(await screen.findByText("r-4812")).toBeTruthy();
});

test("the_log_pops_out_to_a_drawer_and_the_rail_says_where_it_went", async () => {
  await show(page([line({ request: 1 })]));
  fireEvent.click(screen.getByRole("button", { name: "Pop out" }));
  expect(screen.getByText("The log is open along the bottom of the window.")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Bring it back here" }));
  expect(useLogDrawer.getState().out).toBe(false);
});

test("a_request_number_keeps_only_matching_requests_and_no_match_names_the_filter", async () => {
  await show(page([line({ request: 12 }), line({ request: 13 }), line({ request: 112 })]));
  await screen.findByText("r-12");
  fireEvent.change(screen.getByLabelText("Request"), { target: { value: "r-12" } });
  expect(screen.getByText("r-12")).toBeTruthy();
  expect(screen.getByText("r-112")).toBeTruthy();
  expect(screen.queryByText("r-13")).toBeNull();
  fireEvent.change(screen.getByLabelText("Request"), { target: { value: "9999" } });
  expect(screen.getByText(/No hops match. Clear Request 9999 to see more./)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
  expect(screen.getByText("r-13")).toBeTruthy();
});

test("the_log_has_no_axe_violations_open_closed_and_empty", async () => {
  await show(
    page([
      line({ request: 1, node: "gw", wait_ms: 40 }),
      line({
        request: 2,
        level: "error",
        event: "timeout",
        wait_ms: null,
        service_ms: null,
        note: "deadline passed",
      }),
    ]),
  );
  await screen.findByText("r-1");
  filters();
  await expectNoViolations(document.body);
  fireEvent.click(header(1));
  await expectNoViolations(document.body);
});

test("the_log_names_its_colours_and_puts_a_unit_on_every_hop_time", async () => {
  await show();
  expect(await screen.findByLabelText("What the bar colours mean")).toBeTruthy();
  expect(screen.getByText("Waiting in line")).toBeTruthy();
  expect(screen.getByText("Working or on the wire")).toBeTruthy();
  expect(screen.getByText("Failed here")).toBeTruthy();
});

test("a_retry_is_a_separator_between_attempts_not_a_hop_with_a_bar_and_a_time", async () => {
  const retry = line({
    t_ms: 12320,
    level: "info",
    event: "retry",
    wait_ms: null,
    service_ms: null,
    note: "attempt 2 of 3",
  });
  await show(page([line(), retry, line({ t_ms: 12330 })]));
  const sep = (await screen.findByText(/Attempt 2 of 3/)).closest("li") as HTMLElement;
  expect(sep).toBeTruthy();
  // no bar and no time column in it
  expect(sep.querySelector("span[aria-hidden].bg-rule")).toBeNull();
  expect(sep.textContent).not.toMatch(/\bms\b/);
});

test("a_hop_that_ran_in_a_retry_says_which_try_it_was", async () => {
  await show(page([line(), line({ t_ms: 12330, attempt: 2 })]));
  expect(await screen.findByText("try 2")).toBeTruthy();
  expect(screen.queryByText("try 1")).toBeNull();
});
