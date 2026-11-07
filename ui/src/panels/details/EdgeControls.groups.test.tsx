import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, Summary } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useRun } from "../../data/store/run";
import { mockFetch } from "../../test/fetch";
import { EdgeControls } from "./EdgeControls";
import { useGroups } from "./groups/store";

function view(edge: Record<string, unknown> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: {
      nodes: { gw: { type: "gateway" }, app: { type: "service" } },
      edges: [{ from: "gw", to: "app", ...edge }],
    },
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

async function show(edge: Record<string, unknown> = {}) {
  mockFetch([{ json: view(edge) }]);
  await useDraft.getState().load();
  render(<EdgeControls index={0} />);
}

/** What the next edit sends (one request, so one undo step). */
async function edit(act: () => void) {
  const sent = mockFetch([{ json: view() }]);
  act();
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  return sent[0]?.body;
}

/** A group's header: a setup chip can share its name (Circuit breaker), but only a header folds. */
const head = (title: string) => {
  const found = screen
    .getAllByRole("button", { name: new RegExp(`^${title}`) })
    .find((b) => b.hasAttribute("aria-expanded"));
  if (!found) throw new Error(`no group header ${title}`);
  return found;
};

beforeEach(() => {
  useDraft.setState({ view: null, error: null });
  localStorage.clear();
  useGroups.setState({ open: {} }); // every group at its default: folded
});
afterEach(cleanup);

test("a_fresh_wire_shows_the_setups_and_the_basics_with_every_group_folded", async () => {
  await show();
  expect(screen.getByRole("button", { name: "Lossy link" })).toBeTruthy();
  // the basics are open
  expect(screen.getByLabelText("Bandwidth")).toBeTruthy();
  expect(screen.getByLabelText("Request size")).toBeTruthy();
  // the groups are folded and say what they hold
  for (const [title, summary] of [
    ["Connection", "defaults"],
    ["When a call fails", "off"],
    ["Circuit breaker", "off"],
    ["Compression and streaming", "defaults"],
    ["How often it is used", "defaults"],
  ] as [string, string][]) {
    expect(head(title).getAttribute("aria-expanded"), title).toBe("false");
    expect(head(title).textContent, title).toContain(summary);
  }
  expect(screen.queryByLabelText("Packet loss (%)")).toBeNull();
});

test("a_wire_with_retries_says_so_on_the_closed_group_and_marks_it_changed", async () => {
  await show({ retry: {} });
  expect(head("When a call fails").textContent).toContain("3 tries · 50 ms ×2");
  expect(head("When a call fails").textContent).toContain("changed from the default");
  expect(head("Circuit breaker").textContent).not.toContain("changed from the default");
});

test("opening_a_group_shows_its_boxes", async () => {
  await show();
  fireEvent.click(screen.getByRole("button", { name: /^Connection/ }));
  expect(screen.getByLabelText("Packet loss (%)")).toBeTruthy();
  expect(screen.getByLabelText("Secure connection")).toBeTruthy();
});

test("a_quick_setup_is_one_patch_and_shows_as_pressed_once_the_wire_matches", async () => {
  await show();
  const body = await edit(() =>
    fireEvent.click(screen.getByRole("button", { name: "Lossy link" })),
  );
  expect(body).toEqual({ patch: [{ op: "insert", path: ["edges", 0, "loss"], value: 0.01 }] });
});

test("a_setup_that_writes_two_keys_still_sends_one_request", async () => {
  await show();
  const body = await edit(() =>
    fireEvent.click(screen.getByRole("button", { name: "Cold connections" })),
  );
  expect(body).toEqual({
    patch: [
      { op: "insert", path: ["edges", 0, "tls"], value: 1.3 },
      { op: "insert", path: ["edges", 0, "keepalive"], value: 1 },
    ],
  });
});

test("the_matching_setup_is_pressed_and_its_note_is_shown", async () => {
  await show({ loss: 0.01 });
  act(() => useGroups.getState().set("wire.quick", true));
  expect(screen.getByRole("button", { name: "Lossy link" }).getAttribute("aria-pressed")).toBe(
    "true",
  );
  expect(screen.getByRole("button", { name: "Typical" }).getAttribute("aria-pressed")).toBe(
    "false",
  );
  expect(screen.getByText(/one packet in a hundred is lost/)).toBeTruthy();
});

test("typical_removes_what_the_setups_write_in_one_patch", async () => {
  await show({ retry: {}, loss: 0.01 });
  act(() => useGroups.getState().set("wire.quick", true));
  const body = await edit(() => fireEvent.click(screen.getByRole("button", { name: "Typical" })));
  expect(body).toEqual({
    patch: [
      { op: "remove", path: ["edges", 0, "loss"] },
      { op: "remove", path: ["edges", 0, "retry"] },
    ],
  });
});

test("quick_setups_are_open_on_a_wire_with_nothing_set_and_folded_once_it_has_settings", async () => {
  await show();
  const header = () => screen.getByRole("button", { name: /^Quick setups/ });
  expect(header().getAttribute("aria-expanded")).toBe("true");
  cleanup();
  useGroups.setState({ open: {} });
  await show({ loss: 0.01 });
  expect(header().getAttribute("aria-expanded")).toBe("false");
  expect(header().textContent).toContain("Lossy link");
});

test("the_retry_group_draws_its_waits_and_lists_only_what_differs", async () => {
  await show({ retry: { max_attempts: 4 }, abandon: "continue" });
  act(() => useGroups.getState().set("wire.retry", true));
  const waits = screen.getByRole("figure", { name: "Wait before each retry" });
  expect(waits.textContent).toContain("+50 ms");
  expect(waits.textContent).toContain("+100 ms");
  expect(waits.textContent).toContain("+200 ms");
  const table = screen.getByRole("table", { name: "What differs from the default" });
  expect(table.textContent).toContain("Tries");
  expect(table.textContent).not.toContain("First wait");
});

test("the_effect_strip_shows_the_last_run_of_a_wire_that_retries_and_nothing_otherwise", async () => {
  await show({ retry: {} });
  expect(screen.queryByLabelText("Last run over this wire")).toBeNull();
  act(() =>
    useRun.setState({
      latest: {
        id: "s1",
        state: "done",
        generation: 1,
        profile: "p",
        scenario: null,
        source: "draft",
        draft_rev: 0,
        started_ms: 0,
        finished_ms: 1,
        summary: {
          window_s: 30,
          retries: [
            {
              from: "gw",
              to: "app",
              calls: 200,
              retried: 100,
              retries: 150,
              recovered: 50,
              gave_up: 50,
              failed: 60,
            },
          ],
          breakers: [
            {
              from: "gw",
              to: "app",
              replica: 0,
              state_at_start: "closed",
              changes: [],
              open_ms: 15000,
            },
          ],
        } as unknown as Summary,
      },
    }),
  );
  const strip = screen.getByLabelText("Last run over this wire");
  expect(strip.textContent).toContain("Calls failed30%");
  expect(strip.textContent).toContain("Recovered50%");
  expect(strip.textContent).toContain("Breaker open50%");
  useRun.setState({ latest: null });
});

test("typical_on_a_wire_with_nothing_set_sends_nothing", async () => {
  await show();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Typical" }));
  expect(sent).toHaveLength(0);
});

test("reset_puts_a_group_back_in_one_patch", async () => {
  await show({ retry: {}, abandon: "continue", loss: 0.01 });
  fireEvent.click(screen.getByRole("button", { name: /^When a call fails/ }));
  const body = await edit(() =>
    fireEvent.click(screen.getByRole("button", { name: "Reset When a call fails" })),
  );
  expect(body).toEqual({
    patch: [
      { op: "remove", path: ["edges", 0, "retry"] },
      { op: "remove", path: ["edges", 0, "abandon"] },
    ],
  });
});

test("jumping_to_a_setting_opens_its_group_and_focuses_the_box", async () => {
  await show({ retry: {} });
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "backoff" } });
  fireEvent.click(screen.getByRole("button", { name: "First wait" }));
  expect(head("When a call fails").getAttribute("aria-expanded")).toBe("true");
  await vi.waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText("First wait")));
});

test("jumping_to_a_box_that_needs_a_switch_lands_on_the_switch", async () => {
  await show(); // retries off: Tries is not on the page
  fireEvent.change(screen.getByRole("searchbox"), { target: { value: "tries" } });
  fireEvent.click(screen.getByRole("button", { name: "Tries (the first counts)" }));
  await vi.waitFor(() =>
    expect(document.activeElement).toBe(screen.getByLabelText("Retry a failed call")),
  );
});

test("enter_takes_the_first_match_and_escape_clears_the_box", async () => {
  await show();
  const box = screen.getByRole("searchbox") as HTMLInputElement;
  fireEvent.change(box, { target: { value: "loss" } });
  fireEvent.keyDown(box, { key: "Enter" });
  expect(head("Connection").getAttribute("aria-expanded")).toBe("true");
  await vi.waitFor(() =>
    expect(document.activeElement).toBe(screen.getByLabelText("Packet loss (%)")),
  );
  fireEvent.change(box, { target: { value: "zzz" } });
  expect(screen.getByText("No setting matches.")).toBeTruthy();
  fireEvent.keyDown(box, { key: "Escape" });
  expect(box.value).toBe("");
});

test("every_jump_target_is_a_label_on_a_wire_with_everything_switched_on", async () => {
  await show({
    retry: { budget: {} },
    breaker: {},
    abandon: "continue",
  });
  act(() => {
    for (const id of [
      "wire.connection",
      "wire.compress",
      "wire.retry",
      "wire.breaker",
      "wire.usage",
    ])
      useGroups.getState().set(id, true);
  });
  const labels = new Set(
    [...document.querySelectorAll("label")].map((l) => l.textContent?.trim() ?? ""),
  );
  const { WIRE_TARGETS } = await import("./groups/jump");
  // Balance only appears into a component with copies; the rest must be there
  for (const t of WIRE_TARGETS) {
    if (t.label === "Balance") continue;
    expect(labels.has(t.label), t.label).toBe(true);
  }
});

test("the_wire_header_says_what_was_changed_from_the_defaults", async () => {
  await show({ retry: {}, loss: 0.01 });
  expect(screen.getByText("Changed from the defaults: Connection, When a call fails")).toBeTruthy();
});

test("a_wire_with_nothing_set_says_so", async () => {
  await show();
  expect(screen.getByText("Nothing changed from the defaults.")).toBeTruthy();
});
