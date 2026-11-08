import { act, cleanup, fireEvent, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import { mockFetch } from "../../test/fetch";
import { nodeSummary } from "../../test/summary";
import { app, change, show, view } from "./controlsHarness";
import { useGroups } from "./groups/store";

beforeEach(() => {
  localStorage.clear();
  useGroups.setState({ open: {} });
});
afterEach(cleanup);

const heads = () => screen.getAllByRole("button").filter((b) => b.hasAttribute("aria-expanded"));
const headOf = (title: string) => {
  const found = heads().find((b) => b.textContent?.startsWith(title));
  if (!found) throw new Error(`no group ${title}`);
  return found;
};

test("a_service_has_a_cpus_group_that_says_not_limited_until_cores_are_set", async () => {
  await show(nodeSummary(), { ...app });
  act(() => useGroups.setState({ open: {} }));
  expect(headOf("CPUs").textContent).toContain("not limited");
  expect(headOf("CPUs").textContent).not.toContain("changed from the default");
});

test("typing_cores_writes_one_patch_and_the_summary_names_them", async () => {
  await show(nodeSummary(), { ...app });
  expect(await change("Cores", "4")).toEqual({
    patch: [{ op: "insert", path: ["nodes", "app", "cpu"], value: { cores: 4 } }],
  });
});

test("a_component_with_cores_shows_them_when_folded_and_resets_in_one_patch", async () => {
  await show(nodeSummary(), { ...app, cpu: { cores: 0.5 } });
  act(() => useGroups.setState({ open: {} }));
  expect(headOf("CPUs").textContent).toContain("0.5 cores");
  expect(headOf("CPUs").textContent).toContain("changed from the default");
  fireEvent.click(headOf("CPUs"));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Reset CPUs" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["nodes", "app", "cpu"] }] });
});

test("the_cores_box_refuses_zero_and_too_many_and_sends_nothing", async () => {
  await show(nodeSummary(), { ...app, cpu: { cores: 2 } });
  const sent = mockFetch([{ json: view() }]);
  const input = screen.getByLabelText("Cores") as HTMLInputElement;
  for (const typed of ["-1", "5000"]) {
    fireEvent.change(input, { target: { value: typed } });
    fireEvent.blur(input);
    expect(input.value, typed).toBe("2");
  }
  expect(sent).toHaveLength(0);
});

test("a_cache_has_no_cpus_group", async () => {
  await show(nodeSummary(), { type: "cache" }, { name: "c" });
  act(() => useGroups.setState({ open: {} }));
  expect(heads().some((b) => b.textContent?.startsWith("CPUs"))).toBe(false);
});

test("the_changed_line_names_the_cpus_group", async () => {
  await show(nodeSummary(), { ...app, cpu: { cores: 2 }, hold: "call" });
  expect(document.body.textContent).toContain("Changed from the defaults: Server style, CPUs");
});

test("the_run_says_what_the_cores_did_to_it", async () => {
  await show(nodeSummary({ cpu_load: 0.4 }), { ...app, cpu: { cores: 2 } });
  expect(document.body.textContent).toContain("the work used 40% of these cores.");
  cleanup();
  await show(nodeSummary({ cpu_load: 2.5 }), { ...app, cpu: { cores: 2 } });
  expect(document.body.textContent).toContain("the work asked for 2.5 times these cores");
});
