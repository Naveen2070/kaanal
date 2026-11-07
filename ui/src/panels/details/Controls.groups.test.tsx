import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { nodeSummary } from "../../test/summary";
import { app, show, view } from "./controlsHarness";
import { mockFetch } from "../../test/fetch";
import { useGroups } from "./groups/store";

beforeEach(() => {
  localStorage.clear();
  useGroups.setState({ open: {} });
});
afterEach(cleanup);

const head = () => screen.getByRole("button", { name: /^Network card/ });

test("the_network_card_is_folded_and_says_no_limit_when_nothing_is_set", async () => {
  await show(nodeSummary(), { ...app });
  // the harness opens it for the older tests; fold it again for this one
  useGroups.setState({ open: { "node.network": false } });
  fireEvent.click(head());
  fireEvent.click(head());
  expect(head().getAttribute("aria-expanded")).toBe("false");
  expect(head().textContent).toContain("no limit");
  expect(screen.queryByLabelText("Sends up to")).toBeNull();
});

test("a_card_with_a_limit_says_it_when_closed_and_is_marked_changed", async () => {
  await show(nodeSummary(), { ...app, bandwidth: { out: 256 * 1024, in: 512 * 1024 } });
  useGroups.setState({ open: { "node.network": false } });
  fireEvent.click(head());
  fireEvent.click(head());
  expect(head().textContent).toContain("sends up to 256 KB/s");
  expect(head().textContent).toContain("receives up to 512 KB/s");
  expect(head().textContent).toContain("changed from the default");
});

const heads = () => screen.getAllByRole("button").filter((b) => b.hasAttribute("aria-expanded"));
const headOf = (title: string) => {
  const found = heads().find((b) => b.textContent?.startsWith(title));
  if (!found) throw new Error(`no group ${title}`);
  return found;
};
const fold = () => act(() => useGroups.setState({ open: {} }));

test("a_service_has_its_capacity_boxes_open_and_its_extras_folded_with_a_summary", async () => {
  await show(nodeSummary(), { ...app });
  fold();
  expect(screen.getByLabelText("Service time")).toBeTruthy();
  expect(screen.getByLabelText("Queue limit")).toBeTruthy();
  expect(screen.getByLabelText("Timeout")).toBeTruthy();
  expect(headOf("Request limit").textContent).toContain("none");
  expect(headOf("Server style").textContent).toContain("event loop");
  expect(screen.queryByRole("button", { name: "Add a request limit" })).toBeNull();
});

test("a_component_with_copies_has_a_picking_group_and_one_without_does_not", async () => {
  await show(nodeSummary(), { ...app, replicas: 3, balance: "p2c" });
  fold();
  expect(headOf("Picking a copy").textContent).toContain("power of two choices");
  expect(headOf("Picking a copy").textContent).toContain("changed from the default");
  cleanup();
  await show(nodeSummary(), { ...app, replicas: 1 });
  fold();
  expect(heads().some((b) => b.textContent?.startsWith("Picking a copy"))).toBe(false);
});

test("a_request_limit_group_shows_its_rate_and_resets_in_one_patch", async () => {
  await show(nodeSummary(), { ...app, rate_limit: { rps: 100, burst: 20 } });
  fold();
  expect(headOf("Request limit").textContent).toContain("100 / s · burst 20");
  fireEvent.click(headOf("Request limit"));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Reset Request limit" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["nodes", "app", "rate_limit"] }],
  });
});

test("a_balancer_gets_spread_and_detect_in_its_picking_group", async () => {
  await show(nodeSummary(), { type: "load-balancer", spread: "random" }, { name: "lb" });
  fold();
  fireEvent.click(headOf("Picking a copy"));
  expect(screen.getByLabelText("Spread")).toBeTruthy();
  expect(screen.getByLabelText("Notice a downed copy after")).toBeTruthy();
});
