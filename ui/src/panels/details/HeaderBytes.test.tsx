import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vite-plus/test";
import type { Doc, DraftView } from "../../data/api/types";
import { useDraft } from "../../data/store/draft";
import { mockFetch } from "../../test/fetch";
import { headerBytesOf } from "./edgeFields";
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

beforeEach(() => useDraft.setState({ view: null, error: null }));
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
