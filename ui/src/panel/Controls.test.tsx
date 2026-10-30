import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, EdgeDoc, NodeDoc } from "../api/types";
import { FeedContext } from "../canvas/feed";
import { useDraft } from "../store/draft";
import { mockFetch } from "../test/fetch";
import { nodeSummary } from "../test/summary";
import { Controls } from "./Controls";

const app: NodeDoc = { type: "service", concurrency: 12, proc: "98ms", replicas: 2 };
const edges: EdgeDoc[] = [
  { from: "gw", to: "app" },
  { from: "app", to: "db" },
];

function view(over: Partial<DraftView> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: { nodes: { gw: { type: "gateway" }, app, db: { type: "database" } }, edges },
    doc_error: null,
    project_id: "none",
    kind_defaults: {},
    layout: {},
    layout_note: null,
    layout_dirty: false,
    ...over,
  };
}

const topology = {
  nodes: [
    { name: "gw", type: "gateway" },
    { name: "app", type: "service" },
    { name: "db", type: "database" },
  ],
  edges: [
    { from: "gw", to: "app" },
    { from: "app", to: "db" },
  ],
  routes: [],
  endpoints: [],
};

async function show(
  summary = nodeSummary(),
  cfg: NodeDoc = app,
  opts: { applied?: boolean; name?: string } = {},
) {
  const name = opts.name ?? "app";
  // the draft, then (when the panel asks) the running system
  mockFetch([
    { json: view({ doc: { nodes: { [name]: cfg }, edges } }) },
    opts.applied === false ? { status: 409, json: { error: "no system" } } : { json: topology },
  ]);
  await useDraft.getState().load();
  const calls = { onDeleted: vi.fn(), onRenamed: vi.fn(), onChaos: vi.fn() };
  render(
    <FeedContext.Provider
      value={{ node: (n) => (n === name ? summary : undefined), dimmed: false }}
    >
      <Controls name={name} config={cfg} edges={edges} {...calls} />
    </FeedContext.Provider>,
  );
  return calls;
}

beforeEach(() => useDraft.setState({ view: null, error: null, restarted: false }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("every_field_has_a_visible_label_and_hint", async () => {
  await show();
  for (const label of [
    "Name",
    "Slots per instance",
    "Instances",
    "Service time (ms)",
    "Queue limit",
    "Timeout (ms)",
  ]) {
    const input = screen.getByLabelText(label);
    expect(input, label).toBeTruthy();
    const hint = input.getAttribute("aria-describedby");
    if (label !== "Name")
      expect(document.getElementById(hint ?? "")?.textContent, label).toBeTruthy();
  }
});

test("the_retries_control_is_absent_and_so_is_an_error_rate", async () => {
  await show();
  expect(screen.queryByText(/retries/i)).toBeNull();
  expect(screen.queryByLabelText(/error rate/i)).toBeNull();
});

async function change(label: string, value: string) {
  const sent = mockFetch([{ json: view() }]);
  const input = screen.getByLabelText(label);
  fireEvent.change(input, { target: { value } });
  fireEvent.keyDown(input, { key: "Enter" });
  await vi.waitFor(() => expect(sent.length).toBeGreaterThan(0));
  return sent[0]?.body;
}

test("each_field_in_the_panel_patches_its_path", async () => {
  await show();
  expect(await change("Slots per instance", "16")).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "concurrency"], value: 16 }],
  });
  expect(await change("Instances", "3")).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "replicas"], value: 3 }],
  });
  expect(await change("Service time (ms)", "50")).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "proc"], value: "50ms" }],
  });
  expect(await change("Timeout (ms)", "800")).toEqual({
    patch: [{ op: "insert", path: ["nodes", "app", "timeout"], value: "800ms" }],
  });
  expect(await change("Queue limit", "5")).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "concurrency"], value: { limit: 12, queue: 5 } }],
  });
});

test("renaming_sends_rename_node", async () => {
  await show();
  const sent = mockFetch([{ json: view() }]);
  const name = screen.getByLabelText("Name");
  fireEvent.change(name, { target: { value: "api" } });
  fireEvent.blur(name);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "rename_node", from: "app", to: "api" }] });
});

/** Waits until the panel has learned the running system, so the chaos buttons are live. */
async function chaosReady() {
  await vi.waitFor(() =>
    expect((screen.getByRole("button", { name: "Take down" }) as HTMLButtonElement).disabled).toBe(
      false,
    ),
  );
}

test("chaos_requests_name_a_replica_and_a_link_as_objects", async () => {
  await show();
  await chaosReady();
  const sent = mockFetch([{ json: {} }]);
  fireEvent.change(screen.getByLabelText("Break"), { target: { value: "replica:1" } });
  fireEvent.click(screen.getByRole("button", { name: "Take down" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]).toMatchObject({
    url: "/__kaanal/chaos",
    method: "PUT",
    body: { node: "app", replica: 1, down: true },
  });
  fireEvent.change(screen.getByLabelText("Break"), { target: { value: "link:db" } });
  fireEvent.change(screen.getByLabelText("Slow by (ms)"), { target: { value: "20" } });
  fireEvent.click(screen.getAllByRole("button", { name: "Apply" })[0] as HTMLElement);
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({ edge: { from: "app", to: "db" }, latency_ms: 20 });
});

test("a_link_and_instance_options_exist_only_where_they_apply", async () => {
  await show();
  await chaosReady();
  const options = Array.from(screen.getByLabelText("Break").querySelectorAll("option")).map(
    (o) => o.textContent,
  );
  expect(options).toEqual(["the whole node", "instance 0", "instance 1", "the link to db"]);
});

test("the_works_out_to_line_and_the_suggested_fix_follow_the_run", async () => {
  await show(nodeSummary({ saturated: true, visits_per_s: 200 }));
  expect(screen.getByText(/12 at once x 2 instances \/ 98 ms each = 240 visits\/s/)).toBeTruthy();
  expect(screen.getByText(/Add instances/)).toBeTruthy();
});

test("deleting_removes_the_node_and_its_edges_then_tells_the_panel", async () => {
  const { onDeleted } = await show();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Delete component" }));
  await vi.waitFor(() => expect(onDeleted).toHaveBeenCalled());
  expect(sent[0]?.body).toEqual({
    patch: [
      { op: "remove", path: ["nodes", "app"] },
      { op: "remove", path: ["edges", 1] },
      { op: "remove", path: ["edges", 0] },
    ],
  });
});

test("a_database_has_no_service_time_box_and_a_cache_none_either", async () => {
  for (const type of ["database", "cache"]) {
    cleanup();
    await show(nodeSummary(), { type, pool: { size: 20 } }, { name: "x" });
    expect(screen.queryByLabelText("Service time (ms)"), type).toBeNull();
    expect(screen.getByText(/prices its work per operation/), type).toBeTruthy();
  }
});

test("a_value_the_loader_would_reject_is_never_sent_and_the_box_shows_the_real_value", async () => {
  await show();
  const sent = mockFetch([{ json: view() }]);
  const cases: [string, string, string][] = [
    ["Slots per instance", "2.5", "12"],
    ["Slots per instance", "5000000000", "12"],
    ["Slots per instance", "0", "12"],
    ["Instances", "65", "2"],
    ["Instances", "1.5", "2"],
    ["Timeout (ms)", "400000", ""],
    ["Service time (ms)", "-1", "98"],
    ["Queue limit", "-3", "0"],
  ];
  for (const [label, typed, real] of cases) {
    const input = screen.getByLabelText(label) as HTMLInputElement;
    fireEvent.change(input, { target: { value: typed } });
    fireEvent.blur(input);
    expect(input.value, `${label} ${typed}`).toBe(real);
  }
  expect(sent).toHaveLength(0);
});

test("a_tiny_service_time_is_written_without_an_exponent", async () => {
  await show();
  const sent = mockFetch([{ json: view() }]);
  const input = screen.getByLabelText("Service time (ms)");
  fireEvent.change(input, { target: { value: "0.0004" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(JSON.stringify(sent[0]?.body)).not.toMatch(/e-/);
});

test("a_refused_edit_puts_the_old_value_back_in_the_box", async () => {
  await show();
  mockFetch([{ status: 400, json: { error: "no" } }]);
  const input = screen.getByLabelText("Slots per instance") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "20" } });
  fireEvent.blur(input);
  await vi.waitFor(() => expect(input.value).toBe("12"));
});

test("a_rename_tells_the_panel_to_follow_and_a_refused_one_restores_the_name", async () => {
  const calls = await show();
  mockFetch([{ json: view() }]);
  const name = screen.getByLabelText("Name") as HTMLInputElement;
  fireEvent.change(name, { target: { value: "api" } });
  fireEvent.blur(name);
  await vi.waitFor(() => expect(calls.onRenamed).toHaveBeenCalledWith("api"));
  mockFetch([{ status: 400, json: { error: "already a node" } }]);
  fireEvent.change(name, { target: { value: "db" } });
  fireEvent.blur(name);
  await vi.waitFor(() => expect(name.value).toBe("app"));
  expect(calls.onRenamed).toHaveBeenCalledTimes(1);
});

test("chaos_is_blocked_with_a_reason_for_a_component_that_is_only_in_the_draft", async () => {
  await show(nodeSummary(), app, { applied: false });
  await vi.waitFor(() => expect(screen.getByText(/only in the draft/)).toBeTruthy());
  expect((screen.getByRole("button", { name: "Take down" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
});

test("chaos_starts_a_run_and_an_empty_or_bad_box_sends_nothing", async () => {
  const calls = await show();
  await chaosReady();
  const sent = mockFetch([{ json: {} }]);
  const apply = screen.getAllByRole("button", { name: "Apply" });
  fireEvent.change(screen.getByLabelText("Slow by (ms)"), { target: { value: "" } });
  fireEvent.click(apply[0] as HTMLElement);
  fireEvent.change(screen.getByLabelText("Drop (%)"), { target: { value: "150" } });
  fireEvent.click(apply[1] as HTMLElement);
  expect(sent).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "Take down" }));
  await vi.waitFor(() => expect(calls.onChaos).toHaveBeenCalledTimes(1));
});

test("a_chaos_error_is_shown", async () => {
  await show();
  await chaosReady();
  mockFetch([{ status: 404, json: { error: "unknown node 'app'" } }]);
  fireEvent.click(screen.getByRole("button", { name: "Take down" }));
  expect((await screen.findByRole("alert")).textContent).toBe("unknown node 'app'");
});

test("gateway_controls_shows_traffic_routes_and_patches_target", async () => {
  const gwCfg: NodeDoc = { type: "gateway", concurrency: 256 };
  mockFetch([
    {
      json: view({
        doc: {
          nodes: { gw: gwCfg, app, db: { type: "database" } },
          edges,
          endpoints: {
            "GET /": { override: { body: { ok: true } } },
          },
        },
      }),
    },
    { json: topology },
  ]);
  await useDraft.getState().load();
  const calls = { onDeleted: vi.fn(), onRenamed: vi.fn(), onChaos: vi.fn() };
  render(
    <FeedContext.Provider value={{ node: () => undefined, dimmed: false }}>
      <Controls name="gw" config={gwCfg} edges={edges} {...calls} />
    </FeedContext.Provider>,
  );

  expect(screen.getByText("Traffic routes")).toBeTruthy();
  const select = screen.getByLabelText("GET /") as HTMLSelectElement;
  expect(select.value).toBe("");

  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(select, { target: { value: "app" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      {
        op: "insert",
        path: ["endpoints", "GET /", "chain"],
        value: [{ call: "app" }],
      },
    ],
  });
});
