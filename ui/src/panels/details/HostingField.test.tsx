import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView, NodeDoc } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { expectNoViolations } from "../../test/axe";
import { HostingField } from "./HostingField";

const patch = vi.fn(() => Promise.resolve(true));
beforeEach(() => {
  patch.mockClear();
  useDraft.setState({ view: { doc: { nodes: {} } } as unknown as DraftView, patch } as never);
});
afterEach(cleanup);

test("a_database_offers_four_hostings_and_choosing_one_writes_it", async () => {
  const db = { type: "database" } as NodeDoc;
  const { container } = render(<HostingField name="d" config={db} />);
  const select = screen.getByLabelText("Hosting") as HTMLSelectElement;
  expect([...select.options].map((o) => o.value)).toEqual([
    "self",
    "managed",
    "serverless",
    "external",
  ]);
  fireEvent.change(select, { target: { value: "serverless" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "d", "hosting"], value: "serverless" },
  ]);
  await expectNoViolations(container);
});

test("the_engine_shows_for_a_managed_database_only", () => {
  const { rerender } = render(<HostingField name="d" config={{ type: "database" } as NodeDoc} />);
  expect(screen.queryByLabelText(/Engine/)).toBeNull();
  rerender(<HostingField name="d" config={{ type: "database", hosting: "managed" } as NodeDoc} />);
  const engine = screen.getByLabelText(/Engine/) as HTMLSelectElement;
  expect(engine.value).toBe("postgresql");
  fireEvent.change(engine, { target: { value: "aurora-mysql" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "d", "engine"], value: "aurora-mysql" },
  ]);
});

test("a_managed_cache_has_an_engine_and_valkey_is_written_as_its_flavor", () => {
  const { rerender } = render(<HostingField name="c" config={{ type: "cache" } as NodeDoc} />);
  expect(screen.queryByLabelText(/Engine/)).toBeNull();
  rerender(<HostingField name="c" config={{ type: "cache", hosting: "managed" } as NodeDoc} />);
  const engine = screen.getByLabelText(/Engine/) as HTMLSelectElement;
  expect(engine.value).toBe("redis");
  expect([...engine.options].map((o) => o.value)).toEqual(["redis", "valkey", "memcached"]);
  fireEvent.change(engine, { target: { value: "valkey" } });
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "c", "flavor"], value: "valkey" },
  ]);
  rerender(
    <HostingField
      name="c"
      config={{ type: "cache", hosting: "managed", flavor: "valkey" } as NodeDoc}
    />,
  );
  expect((screen.getByLabelText(/Engine/) as HTMLSelectElement).value).toBe("valkey");
  fireEvent.change(screen.getByLabelText(/Engine/), { target: { value: "redis" } });
  expect(patch).toHaveBeenLastCalledWith([
    { op: "set", path: ["nodes", "c", "flavor"], value: "redis" },
  ]);
});

test("a_kind_with_no_choice_shows_no_hosting_box", () => {
  render(<HostingField name="x" config={{ type: "external-api" } as NodeDoc} />);
  expect(screen.queryByLabelText("Hosting")).toBeNull();
});

test("a_cache_offers_self_managed_and_serverless_and_cassandra_says_the_table_has_no_class", () => {
  const { rerender } = render(<HostingField name="c" config={{ type: "cache" } as NodeDoc} />);
  const select = screen.getByLabelText("Hosting") as HTMLSelectElement;
  expect([...select.options].map((o) => o.value)).toEqual(["self", "managed", "serverless"]);
  rerender(
    <HostingField
      name="d"
      config={{ type: "database", flavor: "cassandra", hosting: "managed" } as NodeDoc}
    />,
  );
  expect(screen.queryByLabelText(/Engine/)).toBeNull();
  expect(screen.getByText(/no managed class for a cassandra database/)).toBeTruthy();
});

test("the_engine_choices_follow_the_flavor", () => {
  render(
    <HostingField
      name="d"
      config={{ type: "database", flavor: "mysql", hosting: "managed" } as NodeDoc}
    />,
  );
  const engine = screen.getByLabelText(/Engine/) as HTMLSelectElement;
  expect([...engine.options].map((o) => o.value)).toEqual(["mysql", "aurora-mysql"]);
});

test("a_managed_database_has_a_multi_az_switch_that_aurora_does_not", () => {
  const { rerender } = render(<HostingField name="d" config={{ type: "database" } as NodeDoc} />);
  expect(screen.queryByLabelText(/Multi-AZ/)).toBeNull();
  rerender(<HostingField name="d" config={{ type: "database", hosting: "managed" } as NodeDoc} />);
  const box = screen.getByLabelText(/Multi-AZ/) as HTMLInputElement;
  expect(box.checked).toBe(false);
  fireEvent.click(box);
  expect(patch).toHaveBeenCalledWith([
    { op: "insert", path: ["nodes", "d", "multi_az"], value: true },
  ]);
  rerender(
    <HostingField
      name="d"
      config={{ type: "database", hosting: "managed", multi_az: true } as NodeDoc}
    />,
  );
  fireEvent.click(screen.getByLabelText(/Multi-AZ/));
  expect(patch).toHaveBeenLastCalledWith([{ op: "remove", path: ["nodes", "d", "multi_az"] }]);
  rerender(
    <HostingField
      name="d"
      config={{ type: "database", hosting: "managed", engine: "aurora-mysql" } as NodeDoc}
    />,
  );
  expect(screen.queryByLabelText(/Multi-AZ/)).toBeNull();
});
