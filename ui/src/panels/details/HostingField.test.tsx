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

test("a_kind_with_no_choice_shows_no_hosting_box", () => {
  render(<HostingField name="x" config={{ type: "external-api" } as NodeDoc} />);
  expect(screen.queryByLabelText("Hosting")).toBeNull();
});

test("a_cache_offers_self_managed_and_serverless_and_mongodb_says_the_table_has_no_class", () => {
  const { rerender } = render(<HostingField name="c" config={{ type: "cache" } as NodeDoc} />);
  const select = screen.getByLabelText("Hosting") as HTMLSelectElement;
  expect([...select.options].map((o) => o.value)).toEqual(["self", "managed", "serverless"]);
  rerender(
    <HostingField
      name="d"
      config={{ type: "database", flavor: "mongodb", hosting: "managed" } as NodeDoc}
    />,
  );
  expect(screen.queryByLabelText(/Engine/)).toBeNull();
  expect(screen.getByText(/no managed class for a mongodb database/)).toBeTruthy();
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
