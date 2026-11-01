import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { DraftView } from "../api/types";
import { useDraft } from "../store/draft";
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

beforeEach(() => useDraft.setState({ view: view(), error: null }));
afterEach(cleanup);

test("connect_menu_lists_only_valid_targets", () => {
  expect(targetsFor("gw", ["gw", "api", odd], [{ from: "gw", to: "api" }])).toEqual([odd]);
  render(<ConnectMenu from="gw" />);
  const items = screen.getAllByRole("button").map((b) => b.textContent);
  expect(items).toEqual([odd]); // not itself, not api (already called); odd names work as labels
});

test("nothing_valid_says_why", () => {
  useDraft.setState({
    view: { ...view(), doc: { nodes: { a: { type: "service" } }, edges: [] } },
  });
  render(<ConnectMenu from="a" />);
  expect(screen.getByText(/cannot call itself/)).toBeTruthy();
});

test("a_click_connects_and_a_refusal_is_shown", async () => {
  const connect = vi.fn().mockResolvedValue({ ok: false, message: "the edge was refused" });
  useDraft.setState({ connect });
  render(<ConnectMenu from="gw" />);
  fireEvent.click(screen.getByRole("button", { name: odd }));
  expect(connect).toHaveBeenCalledWith("gw", odd);
  expect((await screen.findByRole("alert")).textContent).toBe("the edge was refused");
});
