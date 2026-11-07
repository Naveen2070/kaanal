import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { Group } from "./Group";
import { useGroups } from "./store";

beforeEach(() => {
  localStorage.clear();
  useGroups.setState({ open: {} });
});
afterEach(cleanup);

const one = (over: Partial<Parameters<typeof Group>[0]> = {}) =>
  render(
    <Group id="wire.retry" title="When a call fails" summary="3 tries · 50 ms ×2" {...over}>
      <p>the boxes</p>
    </Group>,
  );

test("a_group_starts_folded_and_shows_its_summary_not_its_boxes", () => {
  one();
  const head = screen.getByRole("button", { name: /When a call fails/ });
  expect(head.getAttribute("aria-expanded")).toBe("false");
  // shown as pills; the whole line stays in the title and in the button's name for screen readers
  expect(screen.getByText("3 tries")).toBeTruthy();
  expect(screen.getByText("50 ms ×2")).toBeTruthy();
  expect(head.textContent).toContain("3 tries · 50 ms ×2");
  expect(screen.queryByText("the boxes")).toBeNull();
});

test("enter_and_space_open_it_and_the_summary_gives_way_to_the_boxes", () => {
  one();
  const head = screen.getByRole("button", { name: /When a call fails/ });
  fireEvent.click(head); // a native button: Enter and Space are a click
  expect(head.getAttribute("aria-expanded")).toBe("true");
  expect(screen.getByText("the boxes")).toBeTruthy();
  expect(screen.queryByText("3 tries · 50 ms ×2")).toBeNull();
  expect(head.getAttribute("aria-controls")).toBe(screen.getByText("the boxes").parentElement?.id);
});

test("a_group_can_start_open_and_remembers_what_the_person_chose", () => {
  one({ defaultOpen: true });
  expect(screen.getByText("the boxes")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: /When a call fails/ }));
  expect(screen.queryByText("the boxes")).toBeNull();
  expect(JSON.parse(localStorage.getItem("kaanal-groups-open") ?? "{}")).toEqual({
    "wire.retry": false,
  });
});

test("another_mount_of_the_same_group_follows_the_remembered_state", () => {
  useGroups.setState({ open: { "wire.retry": true } });
  one();
  expect(screen.getByText("the boxes")).toBeTruthy();
});

test("a_changed_group_shows_a_dot_and_a_reset_that_runs_once", () => {
  const reset = vi.fn();
  one({ changed: true, onReset: reset, defaultOpen: true });
  expect(screen.getByText("(changed from the default)")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Reset When a call fails" }));
  expect(reset).toHaveBeenCalledTimes(1);
});

test("an_unchanged_group_has_no_dot_and_no_reset", () => {
  one({ defaultOpen: true });
  expect(screen.queryByText("(changed from the default)")).toBeNull();
  expect(screen.queryByRole("button", { name: /^Reset/ })).toBeNull();
});
