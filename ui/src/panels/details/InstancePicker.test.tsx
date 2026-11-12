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
    { name: "aws/db.m5.large", vcpu: 2, memMib: 8192, netMbps: 10000 },
    { name: "aws/cache.m5.large", vcpu: 2, memMib: 6533, netMbps: 10000 },
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
  // the type is written and Cores is filled; with no request size set there is no memory limit to fill
  // (a limit alone is refused: "memory: per_request is required")
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "app", "instance"], value: "aws/m5.large" },
    { op: "insert", path: ["nodes", "app", "cpu"], value: { cores: 2 } },
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
  await waitFor(() => expect(patch).toHaveBeenCalledTimes(3));
  expect(patch).toHaveBeenNthCalledWith(1, [
    { op: "insert", path: ["instances"], value: { "build-box": { cores: 16 } } },
  ]);
  expect(patch).toHaveBeenNthCalledWith(2, [
    { op: "insert", path: ["nodes", "app", "instance"], value: "build-box" },
  ]);
  expect(patch).toHaveBeenNthCalledWith(3, [
    { op: "insert", path: ["nodes", "app", "cpu"], value: { cores: 16 } },
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

test("a_type_also_fills_the_memory_limit_once_a_request_size_is_set", async () => {
  const config = app({ memory: { per_request: 1048576 } });
  draftWith({ nodes: { app: config } });
  render(<InstancePicker name="app" config={config} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "aws" } });
  const box = await screen.findByLabelText("Search instance types");
  await waitFor(() => expect((box as HTMLInputElement).disabled).toBe(false));
  fireEvent.change(box, { target: { value: "m5.large" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "app", "instance"], value: "aws/m5.large" },
    { op: "insert", path: ["nodes", "app", "cpu"], value: { cores: 2 } },
    {
      op: "set",
      path: ["nodes", "app", "memory"],
      value: { per_request: 1048576, limit: 8 * 1024 ** 3 },
    },
  ]);
});

test("the_estimate_warning_stays_while_the_size_per_request_is_the_estimated_one", async () => {
  const GiB = 1024 ** 3;
  const config = app({
    concurrency: 8,
    instance: "aws/m5.large",
    memory: { limit: 8 * GiB, per_request: GiB },
  });
  draftWith({ nodes: { app: config } });
  const { rerender } = render(<InstancePicker name="app" config={config} />);
  expect((await screen.findByRole("status")).textContent).toMatch(
    /an estimate, not a measured value.*1 GiB = \(memory 8 GiB/s,
  );
  // a size the person typed is theirs: no warning
  const own = { ...config, memory: { limit: 8 * GiB, per_request: 3 * 1024 ** 2 } };
  rerender(<InstancePicker name="app" config={own} />);
  expect(screen.queryByText(/an estimate, not a measured value/)).toBeNull();
});

test("only_the_types_the_picked_region_has_are_listed_and_a_missing_one_is_said", async () => {
  const { api } = await import("../../data/api/client");
  const { resetPrices } = await import("../../data/api/prices");
  const { useRegion } = await import("../../data/store/region");
  resetPrices();
  useRegion.setState({ picked: {}, defaults: {}, regions: {}, ready: false });
  vi.spyOn(api, "getPriceRegions").mockResolvedValue({
    defaults: { aws: "us-east-1" },
    regions: { aws: ["us-east-1", "eu-west-1"] },
  });
  // eu-west-1 has m5.large but not c6i.large
  vi.spyOn(api, "getPrices").mockImplementation((_p: string, region: string) =>
    Promise.resolve({
      provider: "aws",
      region,
      prices:
        region === "eu-west-1"
          ? [["aws/m5.large", 107000]]
          : [
              ["aws/c6i.large", 85000],
              ["aws/m5.large", 96000],
            ],
    }),
  );
  const config = app({ instance: "aws/c6i.large" });
  draftWith({ nodes: { app: config } });
  render(<InstancePicker name="app" config={config} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "aws" } });
  const names = async () =>
    [...document.querySelectorAll("datalist option")].map((o) => (o as HTMLOptionElement).value);
  await waitFor(async () => expect(await names()).toEqual(["aws/c6i.large", "aws/m5.large"]));
  expect(screen.queryByText(/may not be offered there/)).toBeNull();
  // the browser picks another region: the list narrows and the node's own type is flagged
  useRegion.getState().pick("aws", "eu-west-1");
  await waitFor(async () => expect(await names()).toEqual(["aws/m5.large"]));
  expect(await screen.findByText(/aws\/c6i\.large has no price listed in eu-west-1/)).toBeTruthy();
  vi.restoreAllMocks();
  useRegion.setState({ picked: {}, defaults: {}, regions: {}, ready: false });
  resetPrices();
});

test("a_managed_database_lists_only_database_classes_and_a_vm_node_never_lists_them", async () => {
  const db = { type: "database", hosting: "managed" } as NodeDoc;
  draftWith({ nodes: { d: db } });
  const { rerender } = render(<InstancePicker name="d" config={db} />);
  // only AWS has database classes in the table
  const select = screen.getByLabelText("Instance type") as HTMLSelectElement;
  expect([...select.options].map((o) => o.value)).toEqual(["", "aws", "custom"]);
  fireEvent.change(select, { target: { value: "aws" } });
  const box = await screen.findByLabelText("Search instance types");
  await waitFor(() => expect((box as HTMLInputElement).disabled).toBe(false));
  const names = () =>
    [...document.querySelectorAll("datalist option")].map((o) => (o as HTMLOptionElement).value);
  await waitFor(() => expect(names()).toEqual(["aws/db.m5.large"]));
  // choosing one writes it and fills the cores, as a VM type does
  fireEvent.change(box, { target: { value: "aws/db.m5.large" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "d", "instance"], value: "aws/db.m5.large" },
    { op: "insert", path: ["nodes", "d", "cpu"], value: { cores: 2 } },
  ]);
  // a self-hosted node lists the VM types and not the classes
  const vm = { type: "database" } as NodeDoc;
  draftWith({ nodes: { d: vm } });
  rerender(<InstancePicker name="d" config={vm} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "aws" } });
  await waitFor(() => expect(names()).toEqual(["aws/c6i.large", "aws/m5.large"]));
});

test("a_managed_cache_lists_cache_classes_and_choosing_one_writes_the_instance_only", async () => {
  const cache = { type: "cache", hosting: "managed" } as NodeDoc;
  draftWith({ nodes: { c: cache } });
  render(<InstancePicker name="c" config={cache} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "aws" } });
  const box = await screen.findByLabelText("Search instance types");
  await waitFor(() => expect((box as HTMLInputElement).disabled).toBe(false));
  const names = () =>
    [...document.querySelectorAll("datalist option")].map((o) => (o as HTMLOptionElement).value);
  await waitFor(() => expect(names()).toEqual(["aws/cache.m5.large"]));
  fireEvent.change(box, { target: { value: "aws/cache.m5.large" } });
  // a cache's hits are fixed times: no cores and no memory limit are written, only the instance
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "c", "instance"], value: "aws/cache.m5.large" },
  ]);
});

test("a_managed_mongodb_database_lists_no_classes", async () => {
  const mongo = { type: "database", flavor: "mongodb", hosting: "managed" } as NodeDoc;
  draftWith({ nodes: { d: mongo } });
  render(<InstancePicker name="d" config={mongo} />);
  fireEvent.change(screen.getByLabelText("Instance type"), { target: { value: "aws" } });
  const box = await screen.findByLabelText("Search instance types");
  await waitFor(() => expect((box as HTMLInputElement).disabled).toBe(false));
  expect(document.querySelectorAll("datalist option").length).toBe(0);
});
