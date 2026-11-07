import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { EdgeControls } from "./EdgeControls";
import { ALL_WIRE_GROUPS } from "./groups/ids";
import { useGroups } from "./groups/store";
import { NodeDetectField } from "./ResilienceFields";

function view(edge1: Record<string, unknown> = {}): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: {
      nodes: {
        gw: { type: "gateway" },
        app: { type: "service", replicas: 3 },
        db: { type: "database" },
      },
      edges: [
        { from: "gw", to: "app" },
        { from: "app", to: "db", ...edge1 },
      ],
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

async function show(index: number, v = view()) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
  render(<EdgeControls index={index} />);
}

/** What the next edit sends. */
async function edit(act: () => void) {
  const sent = mockFetch([{ json: view() }]);
  act();
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  return sent[0]?.body;
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null });
  // the groups start folded; these tests reach into every one of them
  useGroups.setState({ open: Object.fromEntries(ALL_WIRE_GROUPS.map((id) => [id, true])) });
});
afterEach(cleanup);

test("a_wire_starts_with_retries_and_the_breaker_off_and_their_boxes_hidden", async () => {
  await show(1);
  expect((screen.getByLabelText("Retry a failed call") as HTMLInputElement).checked).toBe(false);
  expect(
    (screen.getByLabelText("Stop calling after repeated failures") as HTMLInputElement).checked,
  ).toBe(false);
  expect(screen.queryByLabelText("Tries (the first counts)")).toBeNull();
  expect(screen.queryByLabelText("Calls remembered")).toBeNull();
  expect((screen.getByLabelText("When the caller gives up") as HTMLSelectElement).value).toBe(
    "cancel",
  );
});

test("switching_retries_on_writes_an_empty_retry_at_the_wires_place", async () => {
  await show(1);
  const body = await edit(() => fireEvent.click(screen.getByLabelText("Retry a failed call")));
  expect(body).toEqual({ patch: [{ op: "insert", path: ["edges", 1, "retry"], value: {} }] });
});

test("a_retry_shows_the_defaults_and_commits_a_try_count_inside_retry", async () => {
  await show(1, view({ retry: {} }));
  expect((screen.getByLabelText("Tries (the first counts)") as HTMLInputElement).value).toBe("3");
  expect((screen.getByLabelText("Growth of the wait") as HTMLInputElement).value).toBe("2");
  const box = screen.getByLabelText("Tries (the first counts)");
  const body = await edit(() => {
    fireEvent.change(box, { target: { value: "4" } });
    fireEvent.blur(box);
  });
  expect(body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "retry", "max_attempts"], value: 4 }],
  });
});

test("a_count_outside_the_loaders_range_is_never_sent", async () => {
  await show(1, view({ retry: {} }));
  const sent = mockFetch([{ json: view() }]);
  const box = screen.getByLabelText("Tries (the first counts)") as HTMLInputElement;
  fireEvent.change(box, { target: { value: "9" } });
  fireEvent.blur(box);
  expect(sent).toHaveLength(0);
  expect(box.value).toBe("3");
});

test("the_budget_is_a_toggle_inside_retry_and_its_boxes_follow_it", async () => {
  await show(1, view({ retry: {} }));
  expect(screen.queryByLabelText("Budget (tokens)")).toBeNull();
  const body = await edit(() =>
    fireEvent.click(screen.getByLabelText("Limit retries with a budget")),
  );
  expect(body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "retry", "budget"], value: {} }],
  });
  cleanup();
  await show(1, view({ retry: { budget: { max_tokens: 20 } } }));
  expect((screen.getByLabelText("Budget (tokens)") as HTMLInputElement).value).toBe("20");
  expect((screen.getByLabelText("Refill per success") as HTMLInputElement).value).toBe("0.1");
});

test("the_wait_is_typed_in_a_unit_and_written_as_a_duration", async () => {
  await show(1, view({ retry: {} }));
  fireEvent.change(screen.getByLabelText("First wait unit"), { target: { value: "s" } });
  const box = screen.getByLabelText("First wait");
  const body = await edit(() => {
    fireEvent.change(box, { target: { value: "2" } });
    fireEvent.blur(box);
  });
  expect(body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "retry", "backoff"], value: "2000ms" }],
  });
});

test("abandon_continue_is_written_on_the_wire", async () => {
  await show(1);
  const body = await edit(() =>
    fireEvent.change(screen.getByLabelText("When the caller gives up"), {
      target: { value: "continue" },
    }),
  );
  expect(body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "abandon"], value: "continue" }],
  });
});

test("the_breaker_shows_resilience4js_defaults_and_commits_a_percent_as_a_fraction", async () => {
  await show(1, view({ breaker: {} }));
  expect((screen.getByLabelText("Calls remembered") as HTMLInputElement).value).toBe("100");
  expect((screen.getByLabelText("Open at a failure rate of (%)") as HTMLInputElement).value).toBe(
    "50",
  );
  expect((screen.getByLabelText("Trial calls") as HTMLInputElement).value).toBe("10");
  const box = screen.getByLabelText("Open at a failure rate of (%)");
  const body = await edit(() => {
    fireEvent.change(box, { target: { value: "25" } });
    fireEvent.blur(box);
  });
  expect(body).toEqual({
    patch: [{ op: "insert", path: ["edges", 1, "breaker", "failure_rate"], value: 0.25 }],
  });
});

test("switching_the_breaker_off_removes_it", async () => {
  await show(1, view({ breaker: { window: 20 } }));
  const body = await edit(() =>
    fireEvent.click(screen.getByLabelText("Stop calling after repeated failures")),
  );
  expect(body).toEqual({ patch: [{ op: "remove", path: ["edges", 1, "breaker"] }] });
});

test("detect_is_offered_only_on_a_wire_to_a_component_with_copies", async () => {
  await show(1); // db has one copy
  expect(screen.queryByLabelText("Notice a downed copy after")).toBeNull();
  cleanup();
  await show(0); // app has three
  fireEvent.change(screen.getByLabelText("Notice a downed copy after unit"), {
    target: { value: "s" },
  });
  const box = screen.getByLabelText("Notice a downed copy after");
  const body = await edit(() => {
    fireEvent.change(box, { target: { value: "5" } });
    fireEvent.blur(box);
  });
  expect(body).toEqual({
    patch: [{ op: "insert", path: ["edges", 0, "detect"], value: "5000ms" }],
  });
});

test("a_balancers_detect_shows_its_unit_and_is_written_on_the_node", async () => {
  mockFetch([{ json: view() }]);
  await useDraft.getState().load();
  render(<NodeDetectField name="lb" node={{ type: "load-balancer", detect: "5s" }} />);
  expect((screen.getByLabelText("Notice a downed copy after") as HTMLInputElement).value).toBe("5");
  expect(
    (screen.getByLabelText("Notice a downed copy after unit") as HTMLSelectElement).value,
  ).toBe("s");
  const box = screen.getByLabelText("Notice a downed copy after");
  const body = await edit(() => {
    fireEvent.change(box, { target: { value: "10" } });
    fireEvent.blur(box);
  });
  expect(body).toEqual({
    patch: [{ op: "set", path: ["nodes", "lb", "detect"], value: "10000ms" }],
  });
  const reset = await edit(() => fireEvent.click(screen.getByText("Back to the default")));
  expect(reset).toEqual({ patch: [{ op: "remove", path: ["nodes", "lb", "detect"] }] });
});

test("the_resilience_switches_and_boxes_carry_their_help_buttons", async () => {
  await show(1, view({ retry: {}, breaker: {} }));
  for (const name of [
    "About Retry a failed call",
    "About Tries (the first counts)",
    "About First wait",
    "About Growth of the wait",
    "About Longest wait",
    "About Randomise the wait",
    "About Retry writes too",
    "About Limit retries with a budget",
    "About When the caller gives up",
    "About Stop calling after repeated failures",
    "About Calls remembered",
    "About Stays open for",
  ]) {
    expect(screen.getByRole("button", { name }), name).toBeTruthy();
  }
});

test("a_help_button_beside_a_switch_does_not_flip_the_switch", async () => {
  await show(1);
  const before = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "About Retry a failed call" }));
  expect(before).toHaveLength(0);
  expect(
    (screen.getByRole("checkbox", { name: "Retry a failed call" }) as HTMLInputElement).checked,
  ).toBe(false);
});
