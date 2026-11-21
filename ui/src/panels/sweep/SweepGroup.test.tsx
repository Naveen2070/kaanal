import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NuqsAdapter } from "nuqs/adapters/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type {
  DraftView,
  RunRecord,
  Summary,
  SweepPointDoc,
  SweepRecord,
} from "../../data/api/types";
import { api } from "../../data/api/client";
import { useDraft } from "../../data/store/draft";
import { draftBody, startLoop } from "../../data/store/loop";
import { useMode } from "../../data/store/mode";
import { useRun } from "../../data/store/run";
import { useSweep } from "../../data/store/sweep";
import { RESULTS_GROUP } from "../details/groups/ids";
import { useGroups } from "../details/groups/store";
import { expectNoViolations } from "../../test/axe";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { TOKEN_KEY } from "../../token";
import { Results } from "../details/Results";
import { ReportButtons } from "../details/ResultsExtras";
import { SweepGroup } from "./SweepGroup";

const view = (generation = 1): DraftView => ({
  text: "",
  dirty: false,
  generation,
  lost: [],
  rev: 0,
  boot: "b",
  doc: { nodes: { gw: { type: "gateway" } }, edges: [], traffic: { peak: {} } },
  doc_error: null,
  project_id: "p",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
});

const pt = (k: number, over = false): SweepPointDoc => ({
  k,
  req_s: k * 100,
  requests: 1000,
  goodput_req_s: k * 100,
  error_rate: over ? 0.05 : 0,
  p50_ms: 1,
  p95_ms: 2,
  p99_ms: over ? 2000 : 3,
  cause: over ? "queue" : null,
  nodes: [],
  over: { p99: over, errors: over },
});

const points = [pt(0.25), pt(0.5), pt(1), pt(2, true)];

const done = (generation = 1): SweepRecord => ({
  id: "s7",
  kind: "sweep",
  state: "done",
  generation,
  profile: "peak",
  seed: 7,
  max_factor: 8,
  fingerprint: "abc123def456",
  sweep_points: [],
  sweep: {
    label: "a relative what-if, not a prediction",
    limit: { p99_ms: 1000, error_rate: 0.01 },
    points,
    verdict: { kind: "knee", k: 1, req_s: 100 },
    recovers_at: null,
    stopped_at: null,
    first_to_saturate: { node: "db", k: 2, req_s: 200, analytic_req_s: 180 },
    dominant_cause: "queue",
    warnings: ["the ladder is short"],
  },
});

function seedDone(generation = 1) {
  const r = done(generation);
  useSweep.setState({
    status: "done",
    record: r,
    points: r.sweep?.points ?? [],
    report: "# kept report",
    error: null,
  });
}

const mount = () => render(<SweepGroup />);

const summary = (): Summary => ({
  label: "relative what-if, not a prediction",
  window_s: 30,
  requests: 100,
  failed: 0,
  errors: {},
  warnings: [],
  latency: { p50_ms: 1, p95_ms: 2, p99_ms: 3, max_ms: 4, buckets: [] },
  series: { window_s: 1, done: [], failed: [], p99_ms: [] },
  streams: [],
  nodes: [nodeSummary({ name: "gw" })],
  links: [],
  bottlenecks: [],
});

const runRecord = (): RunRecord => ({
  id: "s1",
  state: "done",
  generation: 1,
  profile: "peak",
  scenario: null,
  source: "draft",
  draft_rev: 0,
  started_ms: 0,
  finished_ms: 1,
  summary: summary(),
});

function mountResults() {
  history.replaceState(null, "", "/__kaanal/studio");
  return render(
    <NuqsAdapter>
      <Results />
    </NuqsAdapter>,
  );
}

beforeEach(() => {
  sessionStorage.setItem(TOKEN_KEY, "tok");
  useMode.setState({ mode: "batch" });
  useGroups.setState({ open: { [RESULTS_GROUP.sweep]: true } });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
  useDraft.setState({ view: view() });
  useSweep.getState().reset();
});
afterEach(() => {
  cleanup();
  useSweep.getState().reset();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  sessionStorage.clear();
});

test("start_sends_the_limits_and_shows_progress", async () => {
  const sent = mockFetch([
    { status: 202, json: { id: "s1" } },
    {
      json: {
        ...done(),
        state: "running",
        sweep: null,
        sweep_points: [pt(0.25), pt(0.5)],
      },
    },
  ]);
  mount();
  fireEvent.change(screen.getByLabelText(/p99 limit/i), { target: { value: "500" } });
  fireEvent.change(screen.getByLabelText(/error limit/i), { target: { value: "2" } });
  fireEvent.click(screen.getByRole("button", { name: "Start" }));
  expect(await screen.findByText(/point 2 of up to \d+/)).toBeTruthy();
  expect(sent[0]?.url).toMatch(/\/sim\/sweep$/);
  // no profile: the server sweeps the applied design's own first one
  expect(sent[0]?.body).toEqual({ p99_ms: 500, error_pct: 2 });
  expect(screen.getByRole("button", { name: "Start" })).toHaveProperty("disabled", true);
  expect(await screen.findByRole("img")).toBeTruthy();
});

test("bad_limits_show_a_message_and_send_nothing", () => {
  const sent = mockFetch([{ status: 202, json: { id: "s1" } }]);
  mount();
  fireEvent.change(screen.getByLabelText(/p99 limit/i), { target: { value: "0" } });
  expect(screen.getByText(/p99 limit must be above 0/i)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Start" }));
  fireEvent.change(screen.getByLabelText(/p99 limit/i), { target: { value: "1000" } });
  fireEvent.change(screen.getByLabelText(/error limit/i), { target: { value: "101" } });
  expect(screen.getByText(/error limit must be 0 to 100/i)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Start" }));
  expect(sent).toHaveLength(0);
});

test("an_empty_or_out_of_range_field_blames_that_field_alone", () => {
  const sent = mockFetch([{ status: 202, json: { id: "s1" } }]);
  mount();
  const p99 = screen.getByLabelText(/p99 limit/i);
  const err = screen.getByLabelText(/error limit/i);
  const start = screen.getByRole("button", { name: "Start" });
  const check = (p99Text: string, errText: string, bad: { p99: boolean; err: boolean }) => {
    fireEvent.change(p99, { target: { value: p99Text } });
    fireEvent.change(err, { target: { value: errText } });
    expect([p99.getAttribute("aria-invalid"), err.getAttribute("aria-invalid")]).toEqual([
      String(bad.p99),
      String(bad.err),
    ]);
    expect(start).toHaveProperty("disabled", bad.p99 || bad.err);
    fireEvent.click(start);
  };
  check("1000", "", { p99: false, err: true }); // Number("") is 0: must not send 0 %
  expect(screen.getByText(/type an error limit/i)).toBeTruthy();
  check(" ", "1", { p99: true, err: false });
  expect(screen.getByText(/type a p99 limit/i)).toBeTruthy();
  check("1e400", "1", { p99: true, err: false }); // Infinity is the p99's fault, not the error's
  check("2e9", "1", { p99: true, err: false }); // over the 1e9 ms cap
  expect(screen.getByText(/at most 1,000,000,000 ms/)).toBeTruthy();
  check("1000", "1e400", { p99: false, err: true });
  check("", "", { p99: true, err: true });
  expect(sent).toHaveLength(0);
  check("1e9", "0", { p99: false, err: false });
  expect(sent).toHaveLength(1);
});

test("the_reason_start_is_off_is_linked_to_the_button", () => {
  useMode.setState({ mode: "live" });
  mount();
  fireEvent.change(screen.getByLabelText(/p99 limit/i), { target: { value: "0" } });
  const why = (screen.getByRole("button", { name: "Start" }).getAttribute("aria-describedby") ?? "")
    .split(" ")
    .map((id) => document.getElementById(id)?.textContent ?? "");
  expect(why).toHaveLength(2);
  expect(why[0]).toMatch(/a live system cannot/);
  expect(why[1]).toMatch(/p99 limit must be above 0/);
});

test("a_dirty_draft_says_the_applied_design_is_swept", () => {
  useDraft.setState({ view: { ...view(), dirty: true, doc: { nodes: {}, edges: [] } } });
  mount();
  expect(screen.getByText("Sweeps the applied design; Apply to include your edits.")).toBeTruthy();
  // the draft's missing profile says nothing of the applied design: the server decides
  expect(screen.getByRole("button", { name: "Start" })).toHaveProperty("disabled", false);
  cleanup();
  useDraft.setState({ view: view() });
  mount();
  expect(screen.queryByText(/Apply to include your edits/)).toBeNull();
});

test("start_waits_for_the_design_to_load_and_says_why", () => {
  useDraft.setState({ view: null });
  const sent = mockFetch([{ status: 202, json: { id: "s1" } }]);
  mount();
  const start = screen.getByRole("button", { name: "Start" });
  expect(start).toHaveProperty("disabled", true);
  const why = document.getElementById(start.getAttribute("aria-describedby") ?? "");
  expect(why?.textContent).toBe("Loading the design; Start is ready once it arrives.");
  fireEvent.click(start);
  expect(sent).toHaveLength(0);
});

test("a_clean_draft_with_no_traffic_disables_start_with_a_note", () => {
  useDraft.setState({
    view: { ...view(), doc: { nodes: { gw: { type: "gateway" } }, edges: [] } },
  });
  mount();
  expect(screen.getByRole("button", { name: "Start" })).toHaveProperty("disabled", true);
  expect(screen.getByText("The design has no traffic profile to sweep.")).toBeTruthy();
});

test("the_limit_line_keeps_the_sent_limit_while_running_and_after_coming_back", async () => {
  mockFetch([
    { status: 202, json: { id: "s1" } },
    { json: { ...done(), state: "running", sweep: null, sweep_points: [pt(0.25), pt(0.5)] } },
  ]);
  mount();
  fireEvent.change(screen.getByLabelText(/p99 limit/i), { target: { value: "500" } });
  fireEvent.click(screen.getByRole("button", { name: "Start" }));
  expect(await screen.findByText("limit p99 500 ms")).toBeTruthy();
  fireEvent.change(screen.getByLabelText(/p99 limit/i), { target: { value: "2000" } });
  expect(screen.getByText("limit p99 500 ms")).toBeTruthy();
  expect(screen.queryByText("limit p99 2,000 ms")).toBeNull();
  cleanup(); // leave the tab and come back
  mount();
  expect(screen.getByLabelText(/p99 limit/i)).toHaveProperty("value", "500");
  expect(await screen.findByText("limit p99 500 ms")).toBeTruthy();
});

test("the_finished_view_has_the_finding_chart_and_buttons", async () => {
  seedDone();
  mount();
  expect(screen.getByText(/knee at 100 req\/s/)).toBeTruthy();
  expect(await screen.findByRole("img")).toBeTruthy();
  expect(screen.getByText(/First to saturate/)).toBeTruthy();
  expect(screen.getByText(/^db at 200 req\/s \(queueing estimate 180/)).toBeTruthy();
  expect(screen.getByText(/Dominant cause/)).toBeTruthy();
  expect(screen.getByText("the ladder is short")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Copy as Markdown" })).toBeTruthy();
  expect(screen.getByRole("button", { name: /Save as a file/ })).toBeTruthy();
});

test("copy_and_save_use_the_kept_report_text", async () => {
  seedDone();
  const writeText = vi.fn(() => Promise.resolve());
  vi.stubGlobal("navigator", { clipboard: { writeText } });
  const blobs: unknown[][] = [];
  vi.stubGlobal(
    "Blob",
    class {
      constructor(parts: unknown[]) {
        blobs.push(parts);
      }
    },
  );
  const create = vi.fn(() => "blob:x");
  vi.stubGlobal("URL", { createObjectURL: create, revokeObjectURL: vi.fn() });
  let name = "";
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
    this: HTMLAnchorElement,
  ) {
    name = this.download;
  });
  mount();
  fireEvent.click(screen.getByRole("button", { name: "Copy as Markdown" }));
  expect(writeText).toHaveBeenCalledWith("# kept report");
  expect(await screen.findByText("Copied")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: /Save as a file/ }));
  expect(blobs).toEqual([["# kept report"]]);
  expect(name).toBe("kaanal-sweep-s7.md");
  expect(screen.getByText("Saved")).toBeTruthy();
});

test("the_run_report_is_saved_as_kaanal_run_id", () => {
  vi.stubGlobal(
    "Blob",
    class {
      constructor(_parts: unknown[]) {}
    },
  );
  vi.stubGlobal("URL", { createObjectURL: () => "blob:x", revokeObjectURL: vi.fn() });
  let name = "";
  vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (
    this: HTMLAnchorElement,
  ) {
    name = this.download;
  });
  render(<ReportButtons runId="s3" s={summary()} />);
  fireEvent.click(screen.getByRole("button", { name: /Save as a file/ }));
  expect(name).toBe("kaanal-run-s3.md");
});

test("live_mode_disables_start_with_a_note", () => {
  useMode.setState({ mode: "live" });
  mount();
  expect(screen.getByRole("button", { name: "Start" })).toHaveProperty("disabled", true);
  expect(screen.getByText(/a sweep steps the load, which a live system cannot/i)).toBeTruthy();
});

test("a_busy_answer_shows_busy_and_starts_nothing", async () => {
  const sent = mockFetch([{ status: 409, json: { error: "a run is in progress" } }]);
  mount();
  fireEvent.click(screen.getByRole("button", { name: "Start" }));
  expect(await screen.findByText(/busy/i)).toBeTruthy();
  expect(sent).toHaveLength(1);
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByText(/a run is in progress/)).toBeNull();
  expect(screen.queryByRole("img")).toBeNull();
});

test("the_auto_run_loop_hitting_a_running_sweep_shows_no_error", async () => {
  useRun.setState({ status: "done", latest: runRecord(), dimmed: false, lastError: null });
  useSweep.setState({
    status: "running",
    record: { ...done(), state: "running", sweep: null },
    points: [pt(0.25)],
  });
  // the real client maps the server's 409 to busy
  const sent = mockFetch([
    { status: 409, json: { error: "a run is already in progress", id: "s7" } },
  ]);
  mountResults();
  let naps = 0;
  const loop = startLoop({
    api: { startRun: (b) => api.startRun(b), getRun: (id) => api.getRun(id) },
    draft: { getView: () => useDraft.getState().view, subscribe: () => () => {} },
    run: {
      set: (patch) => useRun.setState(patch),
      hasLatest: () => useRun.getState().latest !== null,
    },
    sleep: async () => {
      if (++naps >= 3) loop.stop();
    },
    body: draftBody,
  });
  await act(async () => {
    loop.runNow();
    await vi.waitFor(() => expect(naps).toBe(3));
  });
  expect(sent.filter((r) => r.url.endsWith("/sim/run"))).toHaveLength(3);
  // the loop waits, dimmed, as for any busy answer
  expect(useRun.getState().dimmed).toBe(true);
  expect(useRun.getState().lastError).toBeNull();
  expect(useRun.getState().status).not.toBe("blocked");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByText(/could not finish/)).toBeNull();
  expect(screen.getByText(/Sweeping: point 1/)).toBeTruthy();
});

test("a_sweep_from_an_earlier_generation_is_labelled_so", async () => {
  useDraft.setState({ view: view(3) });
  seedDone(2);
  const { container } = mount();
  expect(screen.getByText(/made on an earlier design/i)).toBeTruthy();
  await screen.findByRole("img");
  expect(container.textContent).not.toMatch(/current/i);
});

test("a_sweep_from_a_later_generation_is_not_current_either", () => {
  useDraft.setState({ view: view(1) });
  seedDone(2);
  mount();
  expect(screen.getByText(/made on an earlier design/i)).toBeTruthy();
});

test("a_sweep_from_before_a_server_restart_is_labelled_so", () => {
  // generations restart at 1 with the server: the boot id tells the two first generations apart
  seedDone(1);
  useSweep.setState({ sent: { p99_ms: 1000, error_pct: 1, boot: "old" } });
  mount();
  expect(screen.getByText(/made before the server restarted/i)).toBeTruthy();
  cleanup();
  useSweep.setState({ sent: { p99_ms: 1000, error_pct: 1, boot: "b" } });
  mount();
  expect(screen.queryByText(/restarted|earlier design/i)).toBeNull();
});

test("a_sweep_sent_with_no_boot_known_is_not_called_a_restart", () => {
  seedDone(1);
  useSweep.setState({ sent: { p99_ms: 1000, error_pct: 1, boot: null } });
  mount();
  expect(screen.queryByText(/restarted|earlier design/i)).toBeNull();
});

test("the_group_shows_with_no_finished_run_and_keeps_a_finished_sweep", () => {
  useRun.setState({ status: "blocked", latest: null, dimmed: false, lastError: "no build" });
  seedDone();
  mountResults();
  expect(screen.getByText(/No finished run yet/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Start" })).toBeTruthy();
  expect(screen.getByText(/knee at 100 req\/s/)).toBeTruthy();
  cleanup();
  useRun.setState({ status: "running", latest: null });
  mountResults();
  expect(screen.getByRole("button", { name: "Start" })).toBeTruthy();
});

test("the_group_has_no_axe_violations", async () => {
  seedDone();
  const { container } = mount();
  await screen.findByRole("img");
  await expectNoViolations(container);
});
