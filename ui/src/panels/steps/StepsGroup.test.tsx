import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { Doc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { useGroups } from "../details/groups/store";
import { mockFetch } from "../../test/fetch";
import { view } from "../details/controlsHarness";
import { StepsGroup } from "./StepsGroup";

const doc = (endpoints: unknown): Doc =>
  ({
    nodes: { gw: { type: "gateway" }, api: { type: "service" }, db: { type: "database" } },
    endpoints,
  }) as Doc;

beforeEach(() => {
  useDraft.setState({ view: null, error: null, restarted: false });
  useGroups.setState({ open: { "node.steps": true } });
});
afterEach(cleanup);

async function loaded() {
  mockFetch([{ json: view() }]);
  await useDraft.getState().load();
}

const show = (d: Doc) => render(<StepsGroup doc={d} callable={["api", "db"]} />);

test("a_route_with_a_chain_lists_its_steps_in_order", () => {
  show(doc({ "GET /orders": { chain: ["api", { call: "db", op: "read", repeat: 2 }] } }));
  const list = screen.getByRole("list", { name: "Steps of GET /orders" });
  expect([...list.querySelectorAll("li")].map((l) => l.textContent?.slice(0, 24))).toEqual([
    "call api",
    "call db · read · ×2",
  ]);
});

test("a_route_with_no_chain_says_the_default_runs_and_offers_to_add_a_call", () => {
  show(doc({ "GET /orders": {} }));
  expect(screen.getByText(/the default chain runs/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Use the default chain" })).toBeNull();
  expect(screen.getByRole("button", { name: "Add call" })).toBeTruthy();
});

test("the_route_picker_switches_the_list", () => {
  show(doc({ "GET /a": { chain: ["api"] }, "GET /b": { chain: ["db"] } }));
  expect(screen.getByRole("list", { name: "Steps of GET /a" })).toBeTruthy();
  fireEvent.change(screen.getByRole("combobox", { name: "Route" }), {
    target: { value: "GET /b" },
  });
  expect(screen.getByRole("list", { name: "Steps of GET /b" })).toBeTruthy();
});

test("adding_a_call_is_one_patch_that_appends_it", async () => {
  await loaded();
  await useDraft
    .getState()
    .load()
    .catch(() => {});
  show(doc({ "GET /orders": { chain: ["api"] } }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByRole("combobox", { name: "Component to call" }), {
    target: { value: "db" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add call" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "append", path: ["endpoints", "GET /orders", "chain"], value: "db" }],
  });
});

test("moving_a_step_down_swaps_it_with_the_next_in_one_patch_and_the_ends_are_disabled", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["api", { call: "db", repeat: 2 }] } }));
  expect(
    (screen.getByRole("button", { name: "Move call api up" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  expect(
    (screen.getByRole("button", { name: "Move call db · ×2 down" }) as HTMLButtonElement).disabled,
  ).toBe(true);
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Move call api down" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  const chain = ["endpoints", "GET /orders", "chain"];
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "swap", path: [...chain, 0], with: [...chain, 1] }],
  });
});

test("deleting_a_step_and_using_the_default_chain_are_one_patch_each", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["api", "db"] } }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Delete call db" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["endpoints", "GET /orders", "chain", 1] }],
  });
  fireEvent.click(screen.getByRole("button", { name: "Use the default chain" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({
    patch: [{ op: "remove", path: ["endpoints", "GET /orders", "chain"] }],
  });
});

test("a_chain_in_a_form_the_panel_does_not_read_is_left_alone", () => {
  show(doc({ "GET /orders": { chain: "api" } }));
  expect(screen.getByText(/left as it is/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Add call" })).toBeNull();
});

test("a_step_with_its_own_retry_wears_a_badge_and_the_group_notes_it_wins", () => {
  show(doc({ "GET /orders": { chain: [{ call: "db", retry: { attempts: 3 } }] } }));
  expect(screen.getByText("Retry")).toBeTruthy();
  expect(screen.getByText(/instead of the wire/)).toBeTruthy();
});

test("the_folded_group_summarises_routes_steps_and_retries", () => {
  act(() => useGroups.setState({ open: {} }));
  show(doc({ "GET /orders": { chain: ["api", { call: "db", retry: {} }] } }));
  expect(screen.getByText("1 route")).toBeTruthy();
  expect(screen.getByText("2 steps")).toBeTruthy();
  expect(screen.getByText("1 retry")).toBeTruthy();
});

test("nothing_shows_when_endpoints_is_not_a_map_of_routes", () => {
  const { container } = show(doc("endpoints.yaml"));
  expect(container.firstChild).toBeNull();
});

test("a_call_row_opens_its_fields_and_a_box_edit_is_one_patch", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ call: "db", repeat: 2 }] } }));
  expect(screen.queryByLabelText("Repeat")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /^Edit call db/ }));
  const sent = mockFetch([{ json: view() }]);
  const box = screen.getByRole("spinbutton", { name: /^Repeat/ });
  fireEvent.change(box, { target: { value: "4" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["endpoints", "GET /orders", "chain", 0, "repeat"], value: 4 }],
  });
});

test("the_operation_and_stream_pickers_write_or_clear_their_key", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ call: "db", op: "read" }] } }));
  fireEvent.click(screen.getByRole("button", { name: /^Edit call db/ }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByRole("combobox", { name: "Operation" }), { target: { value: "" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["endpoints", "GET /orders", "chain", 0, "op"] }],
  });
  fireEvent.change(screen.getByRole("combobox", { name: "Streaming" }), {
    target: { value: "response" },
  });
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({
    patch: [
      { op: "insert", path: ["endpoints", "GET /orders", "chain", 0, "stream"], value: "response" },
    ],
  });
});

test("changing_the_callee_of_a_bare_step_rewrites_the_item_and_keeps_other_keys_noted", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["api", { call: "db", with: "w" }] } }));
  fireEvent.click(screen.getByRole("button", { name: "Edit call api" }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getAllByRole("combobox", { name: "Component to call" })[0]!, {
    target: { value: "db" },
  });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["endpoints", "GET /orders", "chain", 0], value: "db" }],
  });
  fireEvent.click(screen.getByRole("button", { name: "Edit call db" }));
  expect(screen.getByText(/kept as it is: with/)).toBeTruthy();
});

test("a_step_without_a_retry_offers_its_own_and_a_bare_one_becomes_a_map", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["db"] } }));
  fireEvent.click(screen.getByRole("button", { name: "Edit call db" }));
  expect(screen.getByText(/Uses the wire.s retry/)).toBeTruthy();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Give this call its own retry" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      {
        op: "set",
        path: ["endpoints", "GET /orders", "chain", 0],
        value: { call: "db", retry: {} },
      },
    ],
  });
});

test("a_step_with_its_own_retry_shows_the_wire_boxes_and_edits_land_under_the_step", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ call: "db", retry: { max_attempts: 4 } }] } }));
  fireEvent.click(screen.getByRole("button", { name: /^Edit call db/ }));
  expect(screen.getAllByText(/instead of the wire/)).toHaveLength(2);
  const sent = mockFetch([{ json: view() }]);
  const tries = screen.getByRole("spinbutton", { name: /^Tries/ });
  expect((tries as HTMLInputElement).value).toBe("4");
  fireEvent.change(tries, { target: { value: "3" } });
  fireEvent.blur(tries);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      {
        op: "set",
        path: ["endpoints", "GET /orders", "chain", 0, "retry", "max_attempts"],
        value: 3,
      },
    ],
  });
  fireEvent.click(screen.getByRole("button", { name: "Use the wire's retry" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({
    patch: [{ op: "remove", path: ["endpoints", "GET /orders", "chain", 0, "retry"] }],
  });
});

test("a_cache_row_opens_to_a_cache_picker_and_the_hit_note", async () => {
  await loaded();
  render(
    <StepsGroup
      doc={doc({ "GET /orders": { chain: [{ cache: "c1" }, "db"] } })}
      callable={["api", "db"]}
      caches={["c1", "c2"]}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Edit cache c1" }));
  expect(screen.getByText(/A hit ends the sequence/)).toBeTruthy();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getAllByRole("combobox", { name: "Cache to look up" })[0]!, {
    target: { value: "c2" },
  });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["endpoints", "GET /orders", "chain", 0, "cache"], value: "c2" }],
  });
});

test("adding_a_cache_lookup_appends_a_cache_step", async () => {
  await loaded();
  render(
    <StepsGroup
      doc={doc({ "GET /orders": { chain: ["db"] } })}
      callable={["api", "db"]}
      caches={["c1"]}
    />,
  );
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Add cache lookup" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "append", path: ["endpoints", "GET /orders", "chain"], value: { cache: "c1" } }],
  });
});

test("without_a_cache_component_there_is_no_cache_button", () => {
  show(doc({ "GET /orders": { chain: ["db"] } }));
  expect(screen.queryByRole("button", { name: "Add cache lookup" })).toBeNull();
});

const par = ["endpoints", "GET /orders", "chain", 0, "parallel"];

test("a_parallel_block_shows_a_list_per_branch_and_the_steps_move_inside_their_branch", async () => {
  await loaded();
  show(
    doc({ "GET /orders": { chain: [{ parallel: ["api", ["db", { call: "api", repeat: 2 }]] }] } }),
  );
  expect(screen.getByRole("list", { name: "Steps of branch 1" })).toBeTruthy();
  const second = screen.getByRole("list", { name: "Steps of branch 2" });
  expect(second.querySelectorAll("li")).toHaveLength(2);
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Move call db down" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "swap", path: [...par, 1, 0], with: [...par, 1, 1] }],
  });
});

test("a_lone_step_in_a_branch_has_no_neighbour_to_swap_with", () => {
  show(doc({ "GET /orders": { chain: [{ parallel: ["api", "db"] }] } }));
  const lone = screen.getByRole("list", { name: "Steps of branch 1" });
  for (const b of lone.querySelectorAll("button[aria-label^='Move']")) {
    expect((b as HTMLButtonElement).disabled).toBe(true);
  }
});

test("adding_to_branches_and_removing_one_are_single_patches", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ parallel: ["api", ["db"]] }] } }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Add call to branch 2" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "append", path: [...par, 1], value: "api" }] });
  fireEvent.click(screen.getByRole("button", { name: "Add call to branch 1" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({
    patch: [{ op: "set", path: [...par, 0], value: ["api", "api"] }],
  });
  fireEvent.click(screen.getByRole("button", { name: "Add branch" }));
  await vi.waitFor(() => expect(sent).toHaveLength(3));
  expect(sent[2]?.body).toEqual({ patch: [{ op: "append", path: par, value: "api" }] });
  fireEvent.click(screen.getByRole("button", { name: "Remove branch 2" }));
  await vi.waitFor(() => expect(sent).toHaveLength(4));
  expect(sent[3]?.body).toEqual({ patch: [{ op: "remove", path: [...par, 1] }] });
});

test("the_last_branch_of_a_block_cannot_be_removed_but_the_block_can_be_deleted", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ parallel: ["api"] }] } }));
  expect(screen.queryByRole("button", { name: /^Remove branch/ })).toBeNull();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Delete parallel · 1 branch" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["endpoints", "GET /orders", "chain", 0] }],
  });
});

test("a_parallel_block_is_added_to_the_chain_with_one_branch", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["db"] } }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Add parallel block" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      {
        op: "append",
        path: ["endpoints", "GET /orders", "chain"],
        value: { parallel: ["api"] },
      },
    ],
  });
});

test("a_call_inside_a_branch_opens_and_edits_in_place", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ parallel: [["db"]] }] } }));
  fireEvent.click(screen.getByRole("button", { name: "Edit call db" }));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByRole("combobox", { name: "Operation" }), {
    target: { value: "read" },
  });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: [...par, 0, 0], value: { call: "db", op: "read" } }],
  });
});

test("alt_arrow_from_a_row_moves_it_and_says_so_and_the_ends_do_nothing", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["api", "db", "api"] } }));
  const chain = ["endpoints", "GET /orders", "chain"];
  const sent = mockFetch([{ json: view() }]);
  const second = screen.getAllByRole("button", { name: "Edit call db" })[0]!;
  fireEvent.keyDown(second, { key: "ArrowUp", altKey: true });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "swap", path: [...chain, 1], with: [...chain, 0] }],
  });
  expect(screen.getByRole("status").textContent).toBe("Moved call db to position 1 of 3");
  const first = screen.getAllByRole("button", { name: "Edit call api" })[0]!;
  fireEvent.keyDown(first, { key: "ArrowUp", altKey: true });
  fireEvent.keyDown(second, { key: "ArrowUp", altKey: false });
  expect(sent).toHaveLength(1);
});

test("focus_goes_to_the_moved_step_and_after_a_delete_to_a_neighbour", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: ["api", "db"] } }));
  mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Move call api down" }));
  await vi.waitFor(() =>
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Edit call db"),
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete call db" }));
  await vi.waitFor(() => expect(screen.getByRole("status").textContent).toBe("Deleted call db"));
  await vi.waitFor(() =>
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Edit call db"),
  );
});

test("a_row_names_its_shortcut_for_assistive_tech", () => {
  show(doc({ "GET /orders": { chain: ["api"] } }));
  expect(
    screen.getByRole("button", { name: "Edit call api" }).getAttribute("aria-keyshortcuts"),
  ).toBe("Alt+ArrowUp Alt+ArrowDown");
});

test("the_open_group_with_every_kind_of_step_has_no_axe_violations", async () => {
  const { expectNoViolations } = await import("../../test/axe");
  const { container } = render(
    <StepsGroup
      doc={doc({
        "GET /orders": {
          chain: [
            "api",
            { call: "db", op: "read", retry: { max_attempts: 3 } },
            { cache: "c1" },
            { parallel: ["api", ["db", "api"]] },
          ],
        },
      })}
      callable={["api", "db"]}
      caches={["c1"]}
    />,
  );
  for (const b of screen.getAllByRole("button", { name: /^Edit / })) fireEvent.click(b);
  await expectNoViolations(container);
}, 20_000); // axe over every kind of step is slow when the machine is busy

test("a_step_the_loader_would_not_accept_is_shown_as_written_and_can_be_deleted_or_moved", async () => {
  await loaded();
  show(doc({ "GET /orders": { chain: [{ weird: 1 }, "db"] } }));
  expect(screen.getByText(/not a step Kaanal can run/)).toBeTruthy();
  expect(screen.getByText('{"weird":1}')).toBeTruthy();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Move A step this panel cannot edit down" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [
      {
        op: "swap",
        path: ["endpoints", "GET /orders", "chain", 0],
        with: ["endpoints", "GET /orders", "chain", 1],
      },
    ],
  });
  fireEvent.click(screen.getByRole("button", { name: "Delete A step this panel cannot edit" }));
  await vi.waitFor(() => expect(sent).toHaveLength(2));
  expect(sent[1]?.body).toEqual({
    patch: [{ op: "remove", path: ["endpoints", "GET /orders", "chain", 0] }],
  });
});

test("owned_routes_are_listed_even_when_the_file_has_no_entry_for_them", () => {
  render(<StepsGroup doc={doc({})} routes={["GET /users"]} callable={["db"]} />);
  expect(screen.getByRole("combobox", { name: "Route" })).toBeTruthy();
  expect(screen.getByText(/the default chain runs/i)).toBeTruthy();
});
