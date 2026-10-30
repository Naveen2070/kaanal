import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
import { useRun } from "../store/run";
import { mockFetch } from "../test/fetch";
import { nodeSummary } from "../test/summary";
import { TopBar } from "./TopBar";

function view(rps: number | null): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: {}, ...(rps === null ? {} : { traffic: { peak: { rps, journeys: [] } } }) },
    doc_error: null,
    project_id: "none",
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
  };
}

async function show(rps: number | null) {
  mockFetch([{ json: view(rps) }]);
  await useDraft.getState().load();
  const onRun = vi.fn();
  render(<TopBar onRun={onRun} />);
  return onRun;
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  useRun.setState({ status: "idle", latest: null, dimmed: false, lastError: null });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("letting_go_of_the_slider_patches_the_traffic_rate_once", async () => {
  await show(300);
  const sent = mockFetch([{ json: view(450) }]);
  const slider = screen.getByLabelText("Offered load");
  fireEvent.change(slider, { target: { value: "450" } });
  expect(sent).toHaveLength(0); // dragging sends nothing
  fireEvent.pointerUp(slider);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["traffic", "peak", "rps"], value: 450 }],
  });
});

test("a_draft_with_no_profile_has_a_disabled_slider_and_dashes", async () => {
  await show(null);
  expect((screen.getByLabelText("Offered load") as HTMLInputElement).disabled).toBe(true);
  expect(screen.getByText(/P99/).nextElementSibling?.textContent).toBe("– ms");
  expect(screen.getByText(/Max threshold/).nextElementSibling?.textContent).toBe("–");
});

test("the_figures_come_from_the_newest_run_and_the_run_button_runs", async () => {
  const onRun = await show(300);
  useRun.setState({
    status: "done",
    latest: {
      id: "s1",
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
        requests: 1000,
        failed: 100,
        errors: {},
        warnings: [],
        latency: { p50_ms: 1, p95_ms: 5, p99_ms: 12.34, max_ms: 20, buckets: [] },
        streams: [],
        nodes: [nodeSummary({ limited: 3, overflowed: 4, saturates_at_req_s: 120.4 })],
        links: [],
        bottlenecks: [0],
      },
    },
  });
  expect(await screen.findByText("12.3 ms")).toBeTruthy();
  expect(screen.getByText("90/s")).toBeTruthy(); // (1000 - 100) / 10 s
  expect(screen.getByText("100")).toBeTruthy(); // errors
  expect(screen.getByText("7")).toBeTruthy(); // dropped: 3 + 4
  expect(screen.getByText("120/s")).toBeTruthy(); // max threshold rounded to integer
  expect(screen.getByTitle("Threshold limit: 120/s")).toBeTruthy(); // the | marker on bar
  expect(screen.getByText("(limit 120/s)")).toBeTruthy();

  // Slider max scales a few thousand from threshold (ceil(2120 / 1000) * 1000 = 3000)
  const slider = screen.getByLabelText("Offered load") as HTMLInputElement;
  expect(Number(slider.max)).toBe(3000);

  fireEvent.click(screen.getByRole("button", { name: "Run" }));
  expect(onRun).toHaveBeenCalledTimes(1);
});

test("a_blocked_run_says_why_in_the_status_line", async () => {
  await show(300);
  useRun.setState({ status: "blocked", lastError: "no entry node", dimmed: true });
  expect((await screen.findByRole("status")).textContent).toBe("no entry node");
});

test("tabbing_onto_the_slider_patches_nothing_and_rounds_to_integer", async () => {
  await show(0.5);
  const slider = screen.getByLabelText("Offered load") as HTMLInputElement;
  const sent = mockFetch([{ json: view(1) }]);
  fireEvent.keyUp(slider, { key: "Tab" });
  fireEvent.pointerUp(slider);
  // Displays only integer, no float
  expect(Number(slider.value)).toBe(1);
  expect(screen.getByText("1/s")).toBeTruthy();
  expect(sent).toHaveLength(0);
});

test("an_arrow_key_patches_and_the_slider_keeps_focus", async () => {
  await show(300);
  const slider = screen.getByLabelText("Offered load") as HTMLInputElement;
  slider.focus();
  const sent = mockFetch([{ json: view(301) }]);
  fireEvent.change(slider, { target: { value: "301" } });
  fireEvent.keyUp(slider, { key: "ArrowRight" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["traffic", "peak", "rps"], value: 301 }],
  });
  // the same element, still focused: a key repeat reaches it
  expect(screen.getByLabelText("Offered load")).toBe(slider);
  expect(document.activeElement).toBe(slider);
});
