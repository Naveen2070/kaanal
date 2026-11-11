import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useMode } from "../../data/store/mode";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { NodeDoc, RunRecord } from "../../data/api/types";
import { FeedContext } from "../../canvas/feed";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { useToasts } from "../../shell/toast/toasts";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { Controls } from "./Controls";
import { app, change, edges, show, topology, view } from "./controlsHarness";

beforeEach(() => {
  useMode.setState({ mode: "batch" });
  useDraft.setState({ view: null, error: null, restarted: false });
  useToasts.setState({ toasts: [] });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("every_field_has_a_visible_label_and_hint", async () => {
  await show();
  for (const label of ["Name", "Service time", "Queue limit", "Timeout"]) {
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

test("each_field_in_the_panel_patches_its_path", async () => {
  await show();
  expect(await change("Service time", "50")).toEqual({
    patch: [{ op: "set", path: ["nodes", "app", "proc"], value: "50ms" }],
  });
  expect(await change("Timeout", "800")).toEqual({
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
  fireEvent.change(screen.getByLabelText("Slow by"), { target: { value: "20" } });
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
    expect(screen.queryByLabelText("Service time"), type).toBeNull();
    expect(screen.getByText(/prices its work per operation/), type).toBeTruthy();
  }
});

test("a_value_the_loader_would_reject_is_never_sent_and_the_box_shows_the_real_value", async () => {
  await show();
  const sent = mockFetch([{ json: view() }]);
  const cases: [string, string, string][] = [
    ["Timeout", "400000", ""],
    ["Service time", "-1", "98"],
    ["Queue limit", "-3", ""], // the node has a bare slot count: no limit, an empty box
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
  const input = screen.getByLabelText("Service time");
  fireEvent.change(input, { target: { value: "0.0004" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(JSON.stringify(sent[0]?.body)).not.toMatch(/e-/);
});

test("a_refused_edit_puts_the_old_value_back_in_the_box", async () => {
  await show();
  mockFetch([{ status: 400, json: { error: "no" } }]);
  const input = screen.getByLabelText("Service time") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "20" } });
  fireEvent.blur(input);
  await vi.waitFor(() => expect(input.value).toBe("98"));
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

test("taking_a_node_down_can_start_part_way_into_the_run", async () => {
  await show();
  await chaosReady();
  const sent = mockFetch([{ json: {} }]);
  const box = screen.getByLabelText("Starting at (seconds)") as HTMLInputElement;
  fireEvent.change(box, { target: { value: "4" } });
  fireEvent.click(screen.getByRole("button", { name: "Take down" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ node: "app", down: true, from_ms: 4000 });
  // empty: from the start, no key; a bad value sends nothing
  fireEvent.change(box, { target: { value: "" } });
  fireEvent.click(screen.getByRole("button", { name: "Take down" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({ node: "app", down: true });
  fireEvent.change(box, { target: { value: "-3" } });
  fireEvent.click(screen.getByRole("button", { name: "Take down" }));
  expect(sent).toHaveLength(2);
  // a link has no start: the box goes away
  fireEvent.change(screen.getByLabelText("Break"), { target: { value: "link:db" } });
  expect(screen.queryByLabelText("Starting at (seconds)")).toBeNull();
});

test("chaos_starts_a_run_and_an_empty_or_bad_box_sends_nothing", async () => {
  const calls = await show();
  await chaosReady();
  const sent = mockFetch([{ json: {} }]);
  const apply = screen.getAllByRole("button", { name: "Apply" });
  fireEvent.change(screen.getByLabelText("Slow by"), { target: { value: "" } });
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
  // a failure is a notice (a toast), not text in the panel
  await vi.waitFor(() =>
    expect(useToasts.getState().toasts.map((t) => t.text)).toContain("unknown node 'app'"),
  );
  expect(screen.queryByRole("alert")).toBeNull();
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
    <FeedContext.Provider
      value={{ node: () => undefined, nodes: [], dimmed: false, peakVisits: 0 }}
    >
      <Controls name="gw" config={gwCfg} edges={edges} {...calls} />
    </FeedContext.Provider>,
  );

  expect(await screen.findByText("Traffic routes")).toBeTruthy();
  const group = screen.getByRole("group", { name: "GET /" });
  expect((within(group).getByLabelText("All services") as HTMLInputElement).checked).toBe(false);

  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(within(group).getByLabelText(/^app/));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      { op: "insert", path: ["endpoints", "GET /", "chain"], value: [{ call: "app" }] },
      ...(edges.some((e) => e.from === "gw" && e.to === "app")
        ? []
        : [{ op: "append", path: ["edges"], value: { from: "gw", to: "app" } }]),
    ],
  });
});

test("an_unapplied_draft_offers_apply_in_the_chaos_panel_and_says_what_chaos_hits", async () => {
  mockFetch([{ json: view({ dirty: true, doc: { nodes: { app }, edges } }) }, { json: topology }]);
  await useDraft.getState().load();
  render(
    <FeedContext.Provider
      value={{ node: () => nodeSummary(), nodes: [], dimmed: false, peakVisits: 0 }}
    >
      <Controls
        name="app"
        config={app}
        edges={edges}
        onDeleted={vi.fn()}
        onRenamed={vi.fn()}
        onChaos={vi.fn()}
      />
    </FeedContext.Provider>,
  );
  // known to the running system: chaos stays available, with the caveat
  expect(await screen.findByText(/does not have the draft's unapplied changes/)).toBeTruthy();
  expect((screen.getByRole("button", { name: "Take down" }) as HTMLButtonElement).disabled).toBe(
    false,
  );
  const sent = mockFetch([
    { json: { ok: true, generation: 2, kept: {}, dropped: {}, flagged: [] } },
    { json: view({ dirty: false }) },
  ]);
  fireEvent.click(screen.getByRole("button", { name: "Apply the draft" }));
  await vi.waitFor(() =>
    expect(sent.map((s) => `${s.method} ${s.url}`)).toContain("POST /__kaanal/draft/apply"),
  );
  await vi.waitFor(() =>
    expect(screen.queryByRole("button", { name: "Apply the draft" })).toBeNull(),
  );
});

test("a_clean_draft_shows_no_apply_button_in_the_chaos_panel", async () => {
  await show();
  await chaosReady();
  expect(screen.queryByRole("button", { name: "Apply the draft" })).toBeNull();
});

test("chaos_chips_disabled_until_apply_with_the_reason", async () => {
  await show(nodeSummary(), app, { applied: false });
  const take = await screen.findByRole("button", { name: "Take down" });
  await vi.waitFor(() => expect(take.hasAttribute("disabled")).toBe(true));
  const why = document.getElementById(take.getAttribute("aria-describedby") ?? "");
  expect(why?.textContent).toMatch(/Apply the draft first/);
});

test("slots_and_copies_are_set_in_the_dock_only_and_the_name_is_last", async () => {
  await show();
  expect(screen.queryByLabelText("Slots")).toBeNull();
  expect(screen.queryByLabelText("Copies")).toBeNull();
  const order = [...document.querySelectorAll("h3")].map((h) => h.textContent);
  // the quick setups first (an untouched component), then Capacity
  expect(order[0]).toBe("Quick setups");
  expect(order[1]).toBe("Capacity");
  expect(order.at(-1)).toBe("This component");
});

test("server_style_is_chosen_per_node_and_written_only_when_not_the_default", async () => {
  await show();
  const select = screen.getByLabelText("Server style") as HTMLSelectElement;
  expect(select.value).toBe("own");
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(select, { target: { value: "call" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["nodes", "app", "hold"], value: "call" }],
  });
  cleanup();
  // already `call`: choosing the default again removes the field
  await show(nodeSummary(), { ...app, hold: "call" });
  const held = screen.getByLabelText("Server style") as HTMLSelectElement;
  expect(held.value).toBe("call");
  const removal = mockFetch([{ json: view() }]);
  fireEvent.change(held, { target: { value: "own" } });
  await vi.waitFor(() => expect(removal).toHaveLength(1));
  expect(removal[0]?.body).toEqual({ patch: [{ op: "remove", path: ["nodes", "app", "hold"] }] });
});

test("server_style_is_offered_only_to_a_node_that_calls_others", async () => {
  // a database calls nothing, so it has no style to choose
  await show(nodeSummary(), { type: "database", flavor: "postgres" }, { name: "db" });
  expect(screen.queryByLabelText("Server style")).toBeNull();
});

test("a_thread_per_request_on_a_node_with_no_limit_also_sets_a_typical_pool", async () => {
  await show(nodeSummary(), { type: "service", proc: "10ms" });
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Server style"), { target: { value: "call" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      { op: "insert", path: ["nodes", "app", "hold"], value: "call" },
      { op: "insert", path: ["nodes", "app", "concurrency"], value: { limit: 200, queue: 100 } },
    ],
  });
});

test("a_chaos_hint_shows_in_the_controls_of_the_component_that_has_chaos", async () => {
  useRun.setState({
    latest: { id: "s1", state: "done", summary: { failed: 0 } } as unknown as RunRecord,
  });
  mockFetch([
    { json: view({ doc: { nodes: { app }, edges } }) },
    { json: { effective: { nodes: { app: { down: true } } } } },
    { json: topology },
  ]);
  await useDraft.getState().load();
  render(
    <FeedContext.Provider
      value={{ node: () => undefined, nodes: [], dimmed: false, peakVisits: 0 }}
    >
      <Controls
        name="app"
        config={app}
        edges={edges}
        onDeleted={vi.fn()}
        onRenamed={vi.fn()}
        onChaos={vi.fn()}
      />
    </FeedContext.Provider>,
  );
  expect(await screen.findByText(/Nothing failed although app is down/)).toBeTruthy();
  useRun.setState({ latest: null });
});

test("a_network_card_written_with_a_unit_shows_the_same_number_as_its_bytes", async () => {
  await show(nodeSummary(), { ...app, bandwidth: "250KB" });
  expect((screen.getByLabelText("Sends up to") as HTMLInputElement).value).toBe("250");
  expect((screen.getByLabelText("Receives up to") as HTMLInputElement).value).toBe("250");
  cleanup();
  await show(nodeSummary(), { ...app, bandwidth: 256_000 });
  expect((screen.getByLabelText("Sends up to") as HTMLInputElement).value).toBe("250");
  cleanup();
  await show(nodeSummary(), { ...app, bandwidth: { out: "1 MB/s" } });
  expect((screen.getByLabelText("Sends up to") as HTMLInputElement).value).toBe("1");
  expect((screen.getByLabelText("Receives up to") as HTMLInputElement).value).toBe("");
  cleanup();
  await show(nodeSummary(), { ...app, bandwidth: "fast" });
  expect((screen.getByLabelText("Sends up to") as HTMLInputElement).value).toBe("");
});
