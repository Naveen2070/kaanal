import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { Doc, DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { headerBytesOf } from "./edgeFields";
import { useGroups } from "./groups/store";
import { HeaderBytes } from "./HeaderBytes";

const view = (doc: Doc): DraftView => ({
  text: "",
  dirty: false,
  generation: 1,
  lost: [],
  rev: 0,
  boot: "b",
  doc,
  doc_error: null,
  project_id: "none",
  project_name: null,
  kind_defaults: {},
  layout: {},
  layout_note: null,
  layout_dirty: false,
  saved: true,
});

async function show(doc: Doc) {
  mockFetch([{ json: view(doc) }]);
  await useDraft.getState().load();
  render(<HeaderBytes />);
}

beforeEach(() => {
  useDraft.setState({ view: null, error: null });
  // the group starts folded; the older tests reach into it
  useGroups.setState({ open: { "doc.headers": true } });
});
afterEach(cleanup);

test("header_bytes_reads_a_size_a_pair_or_the_defaults", () => {
  expect(headerBytesOf({})).toEqual({ request: 600, response: 400 });
  expect(headerBytesOf({ header_bytes: 0 })).toEqual({ request: 0, response: 0 });
  expect(headerBytesOf({ header_bytes: "1KB" })).toEqual({ request: 1024, response: 1024 });
  expect(headerBytesOf({ header_bytes: { request: "2KB" } })).toEqual({
    request: 2048,
    response: 400,
  });
});

test("with_nothing_written_the_boxes_show_the_defaults_and_a_commit_inserts_the_pair", async () => {
  await show({ nodes: {}, edges: [] });
  expect((screen.getByLabelText("Request headers") as HTMLInputElement).value).toBe("600");
  expect((screen.getByLabelText("Response headers") as HTMLInputElement).value).toBe("400");
  const sent = mockFetch([{ json: view({}) }]);
  const box = screen.getByLabelText("Response headers");
  fireEvent.change(box, { target: { value: "1000" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "insert", path: ["header_bytes"], value: { request: 600, response: 1000 } }],
  });
});

test("zero_on_both_sides_is_written_as_a_pair_of_zeros", async () => {
  await show({ nodes: {}, edges: [], header_bytes: { request: 0, response: 400 } });
  const sent = mockFetch([{ json: view({}) }]);
  const box = screen.getByLabelText("Response headers");
  fireEvent.change(box, { target: { value: "0" } });
  fireEvent.blur(box);
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({
    patch: [{ op: "set", path: ["header_bytes"], value: { request: 0, response: 0 } }],
  });
});

test("headers_are_a_folded_group_that_says_its_figures_and_resets_to_the_default", async () => {
  await show({ nodes: {}, edges: [], header_bytes: { request: 1024, response: 0 } });
  act(() => useGroups.setState({ open: { "doc.headers": false } }));
  const head = screen.getByRole("button", { name: /^Headers on every call/ });
  expect(head.getAttribute("aria-expanded")).toBe("false");
  expect(head.textContent).toContain("1 KB request · 0 B response");
  expect(head.textContent).toContain("changed from the default");
  fireEvent.click(head);
  const sent = mockFetch([{ json: view({ nodes: {}, edges: [] }) }]);
  fireEvent.click(screen.getByRole("button", { name: "Reset Headers on every call" }));
  await vi.waitFor(() => expect(sent).toHaveLength(1));
  expect(sent[0]?.body).toEqual({ patch: [{ op: "remove", path: ["header_bytes"] }] });
});

test("with_both_sides_at_zero_the_summary_says_headers_are_off", async () => {
  await show({ nodes: {}, edges: [], header_bytes: 0 });
  act(() => useGroups.setState({ open: { "doc.headers": false } }));
  expect(screen.getByRole("button", { name: /^Headers on every call/ }).textContent).toContain(
    "off",
  );
});

test("untouched_headers_say_the_defaults", async () => {
  await show({ nodes: {}, edges: [] });
  act(() => useGroups.setState({ open: { "doc.headers": false } }));
  expect(screen.getByRole("button", { name: /^Headers on every call/ }).textContent).toContain(
    "600 B request · 400 B response",
  );
});
