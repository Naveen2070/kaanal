import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { expectNoViolations } from "../../test/axe";
import { mockFetch } from "../../test/fetch";
import { useToasts } from "../../shell/toast/toasts";
import { closeTrafficBuilder, openTrafficBuilder, useTrafficBuilder } from "./store";
import { TrafficBuilder } from "./TrafficBuilder";
import { TrafficHost } from "./TrafficHost";

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
        shop: { type: "service" },
        search: { type: "service" },
        db: { type: "database" },
      },
      edges: [{ from: "gw", to: "shop" }],
      endpoints: { "GET /products": { chain: [{ call: "shop" }] } },
      traffic: { peak: { rps: 300, journeys: [{ endpoint: "GET /products", weight: 4 }] } },
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

async function load(v = view()) {
  mockFetch([{ json: v }]);
  await useDraft.getState().load();
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null });
  useToasts.setState({ toasts: [] });
  closeTrafficBuilder();
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

test("it_opens_on_the_active_profile_with_a_row_per_endpoint", async () => {
  await load();
  render(<TrafficBuilder />);
  const dialog = screen.getByRole("dialog", { name: "Traffic builder" });
  expect(dialog.textContent).toContain("Profile “peak”");
  expect(dialog.textContent).toContain("300 requests a second");
  expect((screen.getByLabelText("Path of endpoint 1") as HTMLInputElement).value).toBe("/products");
  expect((screen.getByLabelText("Weight of endpoint 1") as HTMLInputElement).value).toBe("4");
});

test("a_draft_with_no_traffic_starts_with_one_blank_endpoint", async () => {
  await load(view({ traffic: undefined, endpoints: undefined }));
  render(<TrafficBuilder />);
  expect((screen.getByLabelText("Path of endpoint 1") as HTMLInputElement).value).toBe("/");
});

test("method_path_weight_and_payload_sizes_are_edited_per_endpoint", async () => {
  await load();
  render(<TrafficBuilder />);
  fireEvent.change(screen.getByLabelText("Method of endpoint 1"), { target: { value: "POST" } });
  fireEvent.change(screen.getByLabelText("Path of endpoint 1"), { target: { value: "/orders" } });
  fireEvent.change(screen.getByLabelText("Request size of endpoint 1"), {
    target: { value: "4" },
  });
  expect((screen.getByLabelText("Method of endpoint 1") as HTMLSelectElement).value).toBe("POST");
  expect((screen.getByLabelText("Request size of endpoint 1") as HTMLInputElement).value).toBe("4");
});

test("a_size_typed_in_megabytes_is_held_in_kilobytes_and_sent_in_bytes", async () => {
  await load();
  openTrafficBuilder();
  render(<TrafficHost />);
  await screen.findByRole("dialog", { name: "Traffic builder" });
  fireEvent.change(screen.getByLabelText("Response size unit of endpoint 1"), {
    target: { value: "MB" },
  });
  fireEvent.change(screen.getByLabelText("Response size of endpoint 1"), {
    target: { value: "2" },
  });
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Apply to the draft" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(JSON.stringify(sent[0]?.body)).toContain('"resp_bytes":2097152');
});

test("all_services_picks_every_service_and_not_the_database", async () => {
  await load();
  render(<TrafficBuilder />);
  fireEvent.click(screen.getByRole("button", { name: /Services behind endpoint 1/ }));
  fireEvent.click(screen.getByLabelText("All services"));
  const list = screen.getByRole("group", { name: /Services behind gw for endpoint 1/ });
  expect((within(list).getByLabelText(/^shop/) as HTMLInputElement).checked).toBe(true);
  expect((within(list).getByLabelText(/^search/) as HTMLInputElement).checked).toBe(true);
  expect(within(list).queryByLabelText(/^db/)).toBeNull();
  expect(
    screen.getByRole("button", { name: /Services behind endpoint 1: All services \(2\)/ }),
  ).toBeTruthy();
});

test("a_service_with_no_gateway_edge_says_so_and_the_footer_names_what_will_connect", async () => {
  await load();
  render(<TrafficBuilder />);
  fireEvent.click(screen.getByRole("button", { name: /Services behind endpoint 1/ }));
  expect(screen.getByText("(not connected yet)")).toBeTruthy(); // search, not shop
  fireEvent.click(screen.getByLabelText(/^search/));
  expect(screen.getByText(/gw will be connected to search/)).toBeTruthy();
});

test("a_chain_written_by_hand_is_kept_unless_replaced", async () => {
  await load(
    view({
      endpoints: { "GET /products": { chain: [{ call: "shop" }, { cache: "c" }] } },
    }),
  );
  render(<TrafficBuilder />);
  expect(
    screen.getByRole("button", {
      name: /Services behind endpoint 1: Custom chain, kept as written/,
    }),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: /Services behind endpoint 1/ }));
  fireEvent.click(screen.getByRole("button", { name: "Replace it with a choice of services" }));
  expect(
    screen.getByRole("button", { name: /Services behind endpoint 1: Gateway replies itself/ }),
  ).toBeTruthy();
});

test("rows_are_added_and_removed", async () => {
  await load();
  render(<TrafficBuilder />);
  fireEvent.click(screen.getByRole("button", { name: "Add endpoint" }));
  expect(screen.getByLabelText("Path of endpoint 2")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Remove endpoint 1" }));
  expect(screen.queryByLabelText("Path of endpoint 2")).toBeNull();
  expect((screen.getByLabelText("Path of endpoint 1") as HTMLInputElement).value).toBe("/");
});

test("what_is_wrong_is_said_and_apply_waits", async () => {
  await load();
  render(<TrafficBuilder />);
  fireEvent.change(screen.getByLabelText("Path of endpoint 1"), { target: { value: "oops" } });
  expect(screen.getByText(/the path starts with \//)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Apply to the draft" }).hasAttribute("disabled")).toBe(
    true,
  );
});

test("apply_sends_one_patch_with_endpoints_mix_and_edges_then_closes_with_a_notice", async () => {
  await load();
  openTrafficBuilder();
  render(<TrafficHost />);
  await screen.findByRole("dialog", { name: "Traffic builder" });
  fireEvent.change(screen.getByLabelText("Path of endpoint 1"), { target: { value: "/items" } });
  fireEvent.click(screen.getByRole("button", { name: /Services behind endpoint 1/ }));
  fireEvent.click(screen.getByLabelText(/^search/));
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Apply to the draft" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  const body = sent[0]?.body as { patch: { op: string; path: unknown[] }[] };
  expect(body.patch.map((o) => `${o.op} ${o.path.join("/")}`)).toEqual([
    "insert endpoints/GET /items",
    "set traffic/peak/journeys",
    "append edges",
  ]);
  await vi.waitFor(() => expect(useTrafficBuilder.getState().open).toBe(false));
  expect(useToasts.getState().toasts[0]?.text).toMatch(
    /Traffic updated: 1 endpoint, connected gw to /,
  );
});

test("a_refused_edit_keeps_the_dialog_and_the_rows", async () => {
  await load();
  openTrafficBuilder();
  render(<TrafficHost />);
  await screen.findByRole("dialog", { name: "Traffic builder" });
  fireEvent.change(screen.getByLabelText("Path of endpoint 1"), { target: { value: "/x" } });
  mockFetch([{ status: 400, json: { error: "no" } }]);
  fireEvent.click(screen.getByRole("button", { name: "Apply to the draft" }));
  await vi.waitFor(() => expect(useDraft.getState().error).toBe("no"));
  expect(useTrafficBuilder.getState().open).toBe(true);
  expect((screen.getByLabelText("Path of endpoint 1") as HTMLInputElement).value).toBe("/x");
});

test("cancel_and_escape_close_without_writing", async () => {
  await load();
  openTrafficBuilder();
  render(<TrafficHost />);
  const dialog = await screen.findByRole("dialog", { name: "Traffic builder" });
  const sent = mockFetch([{ json: view() }]);
  fireEvent(dialog, new Event("cancel", { cancelable: true }));
  expect(useTrafficBuilder.getState().open).toBe(false);
  expect(sent).toHaveLength(0);
});

test("without_a_gateway_a_service_choice_is_refused_in_words", async () => {
  await load(view({ nodes: { shop: { type: "service" } }, edges: [], endpoints: undefined }));
  render(<TrafficBuilder />);
  fireEvent.click(screen.getByRole("button", { name: /Services behind endpoint 1/ }));
  fireEvent.click(screen.getByLabelText(/^shop/));
  expect(screen.getByText(/Add a gateway first/)).toBeTruthy();
});

test("the_dialog_has_no_axe_violations", async () => {
  await load();
  render(<TrafficBuilder />);
  fireEvent.click(screen.getByRole("button", { name: /Services behind endpoint 1/ }));
  await expectNoViolations(document.body);
});
