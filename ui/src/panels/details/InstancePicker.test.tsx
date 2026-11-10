import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, NodeDoc } from "../../data/api/types";
import type * as Real from "../../data/api/instances";
import { useDraft } from "../../data/store/draft";
import { expectNoViolations } from "../../test/axe";

const table = vi.hoisted(() => ({
  rows: [
    { name: "aws/c6i.large", vcpu: 2, memMib: 4096, netMbps: 12500 },
    { name: "aws/m5.large", vcpu: 2, memMib: 8192, netMbps: 10000 },
    { name: "azure/standard_d2s_v5", vcpu: 2, memMib: 8192, netMbps: 0 },
    { name: "gcp/n2-standard-2", vcpu: 2, memMib: 8192, netMbps: 10000 },
  ],
}));
vi.mock("../../data/api/instances", async () => {
  const real = await vi.importActual<typeof Real>("../../data/api/instances");
  return { ...real, loadInstances: () => Promise.resolve(table.rows) };
});

import { InstancePicker } from "./InstancePicker";

const patch = vi.fn(() => Promise.resolve(true));

function draftWith(doc: unknown) {
  useDraft.setState({ view: { doc } as unknown as DraftView, patch } as never);
}

const app = (extra: Record<string, unknown> = {}): NodeDoc =>
  ({ type: "service", ...extra }) as NodeDoc;

beforeEach(() => {
  patch.mockClear();
  draftWith({ nodes: { app: app() } });
});
afterEach(cleanup);

test("a_cloud_type_is_found_by_provider_and_search_and_written_as_the_nodes_instance", async () => {
  render(<InstancePicker name="app" config={app()} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "aws" } });
  const box = await screen.findByLabelText("Search instance types");
  await waitFor(() => expect((box as HTMLInputElement).disabled).toBe(false));
  fireEvent.change(box, { target: { value: "m5.lar" } });
  expect(patch).not.toHaveBeenCalled(); // not a whole name yet
  fireEvent.change(box, { target: { value: "m5.large" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "app", "instance"], value: "aws/m5.large" },
  ]);
});

test("the_chosen_type_is_described_and_none_takes_the_instance_out", async () => {
  const config = app({ instance: "aws/m5.large" });
  draftWith({ nodes: { app: config } });
  render(<InstancePicker name="app" config={config} />);
  expect((await screen.findAllByText(/2 vCPU, 8 GiB, up to 10 Gbit\/s/)).length).toBeGreaterThan(0);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "" } });
  expect(patch).toHaveBeenCalledWith([{ op: "remove", path: ["nodes", "app", "instance"] }]);
});

test("a_type_with_no_network_figure_says_the_network_is_not_limited", async () => {
  const config = app({ instance: "azure/standard_d2s_v5" });
  draftWith({ nodes: { app: config } });
  render(<InstancePicker name="app" config={config} />);
  expect((await screen.findAllByText(/network not limited/)).length).toBeGreaterThan(0);
});

test("your_own_instance_is_added_with_a_name_and_cores_and_then_chosen_for_the_node", async () => {
  render(<InstancePicker name="app" config={app()} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "custom" } });
  fireEvent.change(screen.getByLabelText("New instance name"), { target: { value: "build-box" } });
  fireEvent.change(screen.getByLabelText("New instance cores"), { target: { value: "16" } });
  fireEvent.click(screen.getByRole("button", { name: "Add instance" }));
  await waitFor(() => expect(patch).toHaveBeenCalledTimes(2));
  expect(patch).toHaveBeenNthCalledWith(1, [
    { op: "insert", path: ["instances"], value: { "build-box": { cores: 16 } } },
  ]);
  expect(patch).toHaveBeenNthCalledWith(2, [
    { op: "insert", path: ["nodes", "app", "instance"], value: "build-box" },
  ]);
});

test("a_bad_name_or_cores_or_a_taken_name_is_said_and_nothing_is_written", async () => {
  draftWith({ instances: { rack: { cores: 4 } }, nodes: { app: app() } });
  render(<InstancePicker name="app" config={app()} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "custom" } });
  const click = (name: string, cores: string) => {
    fireEvent.change(screen.getByLabelText("New instance name"), { target: { value: name } });
    fireEvent.change(screen.getByLabelText("New instance cores"), { target: { value: cores } });
    fireEvent.click(screen.getByRole("button", { name: "Add instance" }));
  };
  click("aws/x", "4");
  expect((await screen.findByRole("alert")).textContent).toMatch(/letters, digits/);
  click("rack", "4");
  expect((await screen.findByRole("alert")).textContent).toMatch(/already one of your instances/);
  click("ok", "5000");
  expect((await screen.findByRole("alert")).textContent).toMatch(/0.001 to 4096/);
  expect(patch).not.toHaveBeenCalled();
});

test("your_own_instance_shows_its_boxes_and_cannot_be_deleted_while_a_node_uses_it", async () => {
  const config = app({ instance: "rack" });
  draftWith({ instances: { rack: { cores: 4, memory: 1073741824 } }, nodes: { app: config } });
  render(<InstancePicker name="app" config={config} />);
  expect(await screen.findByLabelText("rack: cores")).toBeTruthy();
  expect((screen.getByLabelText("rack: memory") as HTMLInputElement).value).toBe("1");
  const del = screen.getByRole("button", { name: "Delete rack" }) as HTMLButtonElement;
  expect(del.disabled).toBe(true);
  expect(del.title).toMatch(/Used by app/);
});

test("has_no_axe_violations", async () => {
  const { container } = render(<InstancePicker name="app" config={app()} />);
  await expectNoViolations(container);
});
