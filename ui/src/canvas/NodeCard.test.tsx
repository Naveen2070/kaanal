import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { NodeProps } from "@xyflow/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { mockFetch } from "../test/fetch";
import { nodeSummary } from "../test/summary";
import { useDraft } from "../store/draft";
import { FeedContext, type StatsFeed } from "./feed";
import { NodeCard } from "./NodeCard";
import { durationMs } from "../panel/fields";

vi.mock("@xyflow/react", () => ({
  Handle: () => null,
  Position: { Left: "left", Right: "right" },
}));

const config = { type: "service", concurrency: 12, proc: "98ms" };

function card(feed: StatsFeed, cfg: Record<string, unknown> = config) {
  const props = { data: { name: "app", kind: "service", config: cfg }, selected: false };
  render(
    <FeedContext.Provider value={feed}>
      <NodeCard {...(props as unknown as NodeProps)} />
    </FeedContext.Provider>,
  );
}

const feedOf = (n: ReturnType<typeof nodeSummary> | undefined, dimmed = false): StatsFeed => ({
  node: (name) => (name === "app" ? n : undefined),
  dimmed,
});

function view(doc: DraftView["doc"]): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc,
    doc_error: null,
    project_id: "none",
    project_name: null,
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    saved: true,
  };
}

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("a_card_with_no_run_shows_dashes", () => {
  card(feedOf(undefined));
  expect(screen.getByText("Busy: no run yet")).toBeTruthy();
  expect(screen.queryByLabelText("Busy")).toBeNull();
  expect(screen.getByText("waiting –")).toBeTruthy();
  expect(screen.queryByText(/saturates near/)).toBeNull();
  expect(screen.queryByText(/Saturated/)).toBeNull();
});

test("a_card_with_a_run_shows_busy_waiting_and_headroom", () => {
  card(feedOf(nodeSummary({ max_queue: 4, saturates_at_req_s: 122.4 })));
  expect(screen.getByLabelText("Busy").textContent).toBe("50%");
  expect(screen.getByText("waiting 4")).toBeTruthy();
  expect(screen.getByText("saturates near 120 req/s")).toBeTruthy();
});

test("a_saturated_node_is_red_and_says_why_in_text_not_only_colour", () => {
  card(feedOf(nodeSummary({ saturated: true, utilization: 1 })));
  expect(screen.getByText(/Saturated: the queue grows/)).toBeTruthy();
  expect(document.querySelector(".border-red-600")).not.toBeNull();
});

test("replicas_show_as_x_n_and_a_dimmed_feed_dims_the_card", () => {
  card(feedOf(nodeSummary(), true), { ...config, replicas: 3 });
  expect(screen.getByText("x 3")).toBeTruthy();
  expect(document.querySelector(".opacity-60")).not.toBeNull();
});

/** The draft store with `doc` loaded from the (mocked) server, as the app does. */
async function loaded(doc: DraftView["doc"]) {
  mockFetch([{ json: view(doc) }]);
  await useDraft.getState().load();
}

test("editing_replicas_in_the_quick_card_patches_the_draft", async () => {
  await loaded({ nodes: { app: config } });
  const sent = mockFetch([{ json: view({ nodes: { app: { ...config, replicas: 4 } } }) }]);
  card(feedOf(undefined));
  const input = screen.getByLabelText("Instances") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "4" } });
  fireEvent.blur(input);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["nodes", "app", "replicas"], value: 4 }],
  });
});

test("editing_the_service_time_sets_proc_and_a_bad_value_sends_nothing", async () => {
  await loaded({ nodes: { app: config } });
  const sent = mockFetch([{ json: view({ nodes: { app: { ...config, proc: "50ms" } } }) }]);
  card(feedOf(undefined));
  const time = screen.getByLabelText("Time (ms)") as HTMLInputElement;
  fireEvent.change(time, { target: { value: "" } });
  fireEvent.blur(time);
  fireEvent.change(time, { target: { value: "-5" } });
  fireEvent.blur(time);
  expect(sent).toHaveLength(0);
  fireEvent.change(time, { target: { value: "50" } });
  fireEvent.keyDown(time, { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "proc"], value: "50ms" }],
  });
});

test("a_distribution_is_not_editable_in_the_quick_card", () => {
  card(feedOf(undefined), { type: "service", proc: { p50: "4ms", p99: "20ms" } });
  expect(screen.queryByLabelText("Time (ms)")).toBeNull();
  expect(durationMs("1.5s")).toBe(1500);
});

// the card reads a feed; if it imported the run store a later live stream would have to rewrite it
const sources = import.meta.glob("./*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

test("the_card_does_not_import_the_run_store", () => {
  for (const f of ["./NodeCard.tsx", "./QuickControls.tsx", "./Sparkline.tsx", "./metrics.ts"]) {
    expect(sources[f], f).toBeDefined();
    expect(sources[f], f).not.toMatch(/store\/run|useRun/);
  }
});
