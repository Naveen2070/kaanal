import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { EdgeControls } from "./EdgeControls";

function view(over: Partial<DraftView["doc"]> = {}): DraftView {
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
        { from: "app", to: "db", net: "4ms", bandwidth: 2048 },
      ],
      ...over,
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

beforeEach(() => useDraft.setState({ view: null, error: null }));
afterEach(cleanup);

test("a_wire_shows_its_ends_and_its_current_figures", async () => {
  await show(1);
  expect(screen.getByText("app calls db over this wire.")).toBeTruthy();
  expect((screen.getByLabelText("Network latency (ms)") as HTMLInputElement).value).toBe("4");
  expect((screen.getByLabelText("Bandwidth (KB/s)") as HTMLInputElement).value).toBe("2");
  expect((screen.getByLabelText("Calls per request") as HTMLInputElement).value).toBe("1");
  expect((screen.getByLabelText("Share of requests (%)") as HTMLInputElement).value).toBe("100");
});

test("a_committed_latency_is_one_patch_at_the_wires_place", async () => {
  await show(1);
  const sent = mockFetch([{ json: view() }]);
  const box = screen.getByLabelText("Network latency (ms)");
  fireEvent.change(box, { target: { value: "12" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["edges", 1, "net"], value: "12ms" }],
  });
});

test("a_value_the_loader_would_refuse_is_never_sent", async () => {
  await show(1);
  const sent = mockFetch([{ json: view() }]);
  const share = screen.getByLabelText("Share of requests (%)") as HTMLInputElement;
  fireEvent.change(share, { target: { value: "150" } });
  fireEvent.blur(share);
  const calls = screen.getByLabelText("Calls per request") as HTMLInputElement;
  fireEvent.change(calls, { target: { value: "0" } });
  fireEvent.blur(calls);
  expect(sent).toHaveLength(0);
  expect(share.value).toBe("100"); // the box shows what is really set
});

test("a_wire_out_of_a_gateway_has_no_share_and_says_why", async () => {
  await show(0);
  expect(screen.queryByLabelText("Share of requests (%)")).toBeNull();
  expect(screen.getByText(/out of a gateway/)).toBeTruthy();
});

test("balance_is_offered_only_into_a_component_with_copies_and_the_limit_can_be_removed", async () => {
  await show(0); // into app, which has 3 copies
  expect(screen.getByLabelText("Balance")).toBeTruthy();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.change(screen.getByLabelText("Balance"), { target: { value: "random" } });
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["edges", 0, "balance"], value: "random" }],
  });
  cleanup();

  await show(1); // into db, one copy
  expect(screen.queryByLabelText("Balance")).toBeNull();
  const more = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Remove the bandwidth limit" }));
  await vi.waitFor(() => expect(more).toHaveLength(1));
  expect(more[0]?.body).toEqual({
    patch: [{ op: "remove", path: ["edges", 1, "bandwidth"] }],
  });
});

test("a_latency_spread_is_not_editable_here", async () => {
  await show(
    1,
    view({
      edges: [
        { from: "app", to: "db" },
        { from: "app", to: "db", net: { p50: "4ms", p99: "9ms" } },
      ],
    }),
  );
  expect(screen.queryByLabelText("Network latency (ms)")).toBeNull();
  expect(screen.getByText(/spread of values/)).toBeTruthy();
});
