import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../data/api/types";
import { useDraft } from "../data/store/draft";
import { mockFetch } from "../test/fetch";
import { useToasts } from "../shell/toast/toasts";
import { ConnectMenu, targetsFor } from "./ConnectMenu";

const odd = "db #1/ü x";
function view(): DraftView {
  return {
    text: "",
    dirty: false,
    generation: 1,
    lost: [],
    rev: 0,
    boot: "b",
    doc: {
      nodes: { gw: { type: "gateway" }, api: { type: "service" }, [odd]: { type: "database" } },
      edges: [{ from: "gw", to: "api" }],
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

beforeEach(() => {
  useDraft.setState({ view: view(), error: null });
  useToasts.setState({ toasts: [] });
});
afterEach(cleanup);

test("connect_menu_lists_only_valid_targets", () => {
  const nodes = view().doc?.nodes ?? {};
  // a gateway may call services, not a database
  expect(targetsFor("gw", nodes, [])).toEqual(["api"]);
  expect(targetsFor("api", nodes, [])).toEqual([odd]);
  render(<ConnectMenu from="api" />);
  const items = within(screen.getByRole("list", { name: "Connect api to" }))
    .getAllByRole("button")
    .map((b) => b.textContent);
  expect(items).toEqual([odd]); // not a gateway (a service cannot call one); odd names work as labels
});

test("a_service_is_never_offered_a_gateway", () => {
  const nodes = { gw: { type: "gateway" }, s: { type: "service" }, d: { type: "database" } };
  expect(targetsFor("s", nodes, [])).toEqual(["d"]);
  expect(targetsFor("d", nodes, [])).toEqual([]);
});

test("nothing_valid_lists_nothing_and_does_not_apologise", () => {
  useDraft.setState({
    view: { ...view(), doc: { nodes: { a: { type: "service" } }, edges: [] } },
  });
  render(<ConnectMenu from="a" />);
  expect(screen.queryByRole("list")).toBeNull();
  expect(screen.queryByText(/Nothing to connect/)).toBeNull();
});

test("a_click_connects_and_a_refusal_is_shown", async () => {
  const connect = vi.fn().mockResolvedValue({ ok: false, message: "the edge was refused" });
  useDraft.setState({ connect });
  render(<ConnectMenu from="api" />);
  fireEvent.click(screen.getByRole("button", { name: odd }));
  expect(connect).toHaveBeenCalledWith("api", odd);
  await vi.waitFor(() =>
    expect(useToasts.getState().toasts.map((t) => t.text)).toContain("the edge was refused"),
  );
});

test("each_wire_leaving_the_component_has_a_delete_button_that_removes_that_entry", async () => {
  mockFetch([{ json: view() }]);
  await useDraft.getState().load();
  render(<ConnectMenu from="gw" />);
  expect(screen.getByText("gw to api")).toBeTruthy();
  const sent = mockFetch([{ json: view() }]);
  fireEvent.click(screen.getByRole("button", { name: "Delete the wire from gw to api" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["edges", 0] }] });
});

test("a_component_with_no_outgoing_wires_lists_none", () => {
  render(<ConnectMenu from="api" />);
  expect(screen.queryByRole("list", { name: "Wires from api" })).toBeNull();
});

test("wiring_note_explains_auth_wires_and_a_gateway_that_forwards_nowhere", async () => {
  const { wiringNote } = await import("./ConnectMenu");
  const nodes = { gw: { type: "gateway" }, auth: { type: "auth" }, api: { type: "service" } };
  // auth: always the note about the login route
  expect(wiringNote("auth", nodes, [])).toContain("login route");
  // a gateway wired only to auth stops there; to a service it does not
  expect(wiringNote("gw", nodes, [{ from: "gw", to: "auth" }])).toContain("stop at the gateway");
  expect(
    wiringNote("gw", nodes, [
      { from: "gw", to: "auth" },
      { from: "gw", to: "api" },
    ]),
  ).toBeNull();
  // nothing wired yet, or any other kind: no note
  expect(wiringNote("gw", nodes, [])).toBeNull();
  expect(wiringNote("api", nodes, [])).toBeNull();
});

test("a_service_may_connect_to_storage_and_storage_connects_to_nothing", () => {
  const nodes = {
    gw: { type: "gateway" },
    s: { type: "service" },
    m: { type: "monolith" },
    a: { type: "auth" },
    lb: { type: "load-balancer" },
    st: { type: "storage" },
    d: { type: "database" },
  };
  for (const from of ["s", "m", "a", "lb"]) expect(targetsFor(from, nodes, [])).toContain("st");
  expect(targetsFor("gw", nodes, [])).not.toContain("st");
  expect(targetsFor("st", nodes, [])).toEqual([]);
});
